#!/usr/bin/env bash
# Called by package-linux-release.sh with the fully licensed archive staging tree.
set -euo pipefail
package_dir="$1"
out_dir="$2"
version="$3"
deb_version="${version#v}"
[[ "$deb_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.+~][0-9A-Za-z.+~]*)?$ ]] || {
  echo "Use a Debian-compatible release version, for example v0.1.0." >&2; exit 2;
}
[[ "$(uname -m)" == x86_64 ]] || { echo "Linux installers require x86_64." >&2; exit 2; }
# Ubuntu's librtlsdr2 provides SONAME 2. A developer's /usr/local build may
# provide SONAME 0 instead, which apt cannot satisfy with the declared package.
for binary in "$package_dir"/bin/*; do
  if readelf -d "$binary" | grep -q 'Shared library: \[librtlsdr.so.0\]'; then
    echo "Rebuild against Ubuntu librtlsdr-dev, not /usr/local's librtlsdr.so.0." >&2
    exit 1
  fi
done
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
app="$stage/EZ-SDR.AppDir"
deb="$stage/deb"
doc="usr/share/doc/ez-sdr"
mkdir -p "$app/usr/bin" "$app/usr/lib" "$app/$doc" "$deb/usr/bin" "$deb/$doc" "$deb/DEBIAN"
for root in "$app" "$deb"; do
  install -m 0755 "$package_dir"/bin/* "$root/usr/bin/"
  cp -a "$package_dir/LICENSES" "$root/$doc/"
  install -m 0644 "$package_dir"/{DISTRIBUTION.md,LICENSE.md,SOURCE-COMMIT.txt,NATIVE-LIBRARIES.txt,SOURCE-CODE.tar.gz} "$root/$doc/"
  install -Dm0644 packaging/linux/ez-sdr.desktop "$root/usr/share/applications/ez-sdr.desktop"
  install -Dm0644 packaging/linux/ez-sdr.svg "$root/usr/share/icons/hicolor/scalable/apps/ez-sdr.svg"
done

# Debian resolves libraries through apt. CPAL and the window backend also dlopen
# audio/display libraries, so ldd alone cannot generate a complete Depends field.
cat > "$deb/DEBIAN/control" <<CONTROL
Package: ez-sdr
Version: $deb_version
Section: hamradio
Priority: optional
Architecture: amd64
Maintainer: EZ-SDR contributors <noreply@github.com>
Homepage: https://github.com/Lupc9102/ez-sdr
Depends: libc6 (>= 2.39), libgcc-s1, libasound2t64, libpulse0, librtlsdr2, libusb-1.0-0, libudev1, libcap2, libgl1, libegl1, libx11-6, libxcursor1, libxi6, libxrandr2, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1
Installed-Size: $(du -sk "$deb/usr" | cut -f1)
Description: Software defined radio desktop application
 Receive and inspect radio signals, ADS-B aircraft and satellite imagery.
 Includes the EZ-SDR desktop GUI, headless daemon, dump1090 and matching source.
CONTROL
cp LICENSES/THIRD-PARTY-NOTICES.md "$deb/$doc/copyright"
(cd "$deb" && find usr -type f -print0 | sort -z | xargs -0 md5sum > DEBIAN/md5sums)
dpkg-deb --root-owner-group --build "$deb" "$out_dir/ez-sdr_${deb_version}_amd64.deb"

# Bundle receiver/audio libraries, including PulseAudio's private library and
# codec dependencies. Keep libc, the display/GL driver stack and system services
# supplied by the host to avoid mixing incompatible drivers or libc versions.
declare -A bundled=()
declare -A source_packages=()
queue=("$package_dir"/bin/* /usr/lib/x86_64-linux-gnu/libpulse.so.0)
for ((index=0; index<${#queue[@]}; index++)); do
  while read -r soname arrow path rest; do
    [[ "$arrow" == '=>' ]] || continue
    [[ "$path" != not ]] || { echo "Missing library: $soname" >&2; exit 1; }
    case "$soname" in
      libc.so.*|libm.so.*|libdl.so.*|libpthread.so.*|librt.so.*|libresolv.so.*|libgcc_s.so.*|libudev.so.*|libcap.so.*|libsystemd.so.*|libdbus-1.so.*|libapparmor.so.*|libX*.so.*|libxcb*.so.*|libbsd.so.*|libmd.so.*|libgcrypt.so.*|libgpg-error.so.*|liblz4.so.*|liblzma.so.*|libzstd.so.*) continue ;;
    esac
    [[ -z "${bundled[$soname]:-}" ]] || continue
    bundled[$soname]=1
    install -m0644 "$path" "$app/usr/lib/$soname"
    queue+=("$path")
    real="$(readlink -f "$path")"
    owner="$(dpkg-query -S "$real" 2>/dev/null | head -1 | sed 's/: \/.*//')"
    [[ -n "$owner" ]] || { echo "Bundled library must come from an Ubuntu package: $real" >&2; exit 1; }
    source_package="$(dpkg-query -W -f='${source:Package}=${source:Version}' "$owner")"
    source_packages[$source_package]=1
    install -Dm0644 "/usr/share/doc/${owner%%:*}/copyright" "$app/$doc/LICENSES/native-${owner%%:*}.txt"
  done < <(ldd "${queue[$index]}")
done
# libpulse itself is dlopened rather than linked by the application.
install -m0644 /usr/lib/x86_64-linux-gnu/libpulse.so.0 "$app/usr/lib/libpulse.so.0"
source_packages["$(dpkg-query -W -f='${source:Package}=${source:Version}' libpulse0:amd64)"]=1
install -Dm0644 /usr/share/doc/libpulse0/copyright "$app/$doc/LICENSES/native-libpulse0.txt"
cp -a /usr/share/alsa "$app/usr/share/"
# Hardware profiles/configuration are split into separate Ubuntu packages.
# Preserve their notices and sources too, not just the libasound binary's.
for config_package in libasound2-data alsa-topology-conf alsa-ucm-conf; do
  if [[ "$(dpkg-query -W -f='${db:Status-Status}' "$config_package" 2>/dev/null || true)" == installed ]]; then
    source_packages["$(dpkg-query -W -f='${source:Package}=${source:Version}' "$config_package")"]=1
    install -Dm0644 "/usr/share/doc/$config_package/copyright" "$app/$doc/LICENSES/native-$config_package.txt"
  fi
done

# Include the exact Ubuntu source packages for every bundled native library.
# Use an isolated apt index; no root access or changes to the builder are needed.
mkdir -p "$stage/lists/partial" "$app/$doc/native-source"
cat > "$stage/sources.list" <<'SOURCES'
deb-src [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://archive.ubuntu.com/ubuntu noble main universe
deb-src [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://archive.ubuntu.com/ubuntu noble-updates main universe
deb-src [signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://security.ubuntu.com/ubuntu noble-security main universe
SOURCES
apt_options=(-o "Dir::Etc::sourcelist=$stage/sources.list" -o Dir::Etc::sourceparts=- -o "Dir::State::lists=$stage/lists" -o APT::Get::List-Cleanup=0)
apt-get "${apt_options[@]}" update
(cd "$app/$doc/native-source" && apt-get "${apt_options[@]}" source --download-only --only-source "${!source_packages[@]}")
{
  echo 'AppImage bundled native source packages:'
  printf '%s\n' "${!source_packages[@]}" | sort
  echo 'AppImage runtime: AppImage/type2-runtime 20251108 (MIT); source: https://github.com/AppImage/type2-runtime/tree/20251108'
} >> "$app/$doc/NATIVE-LIBRARIES.txt"
install -m0755 packaging/linux/AppRun "$app/AppRun"
cp packaging/linux/ez-sdr.desktop packaging/linux/ez-sdr.svg "$app/"
ln -s ez-sdr.svg "$app/.DirIcon"
desktop-file-validate "$app/ez-sdr.desktop"

# Fixed, verified tool/runtime releases; building never requires FUSE.
fetch() { curl --fail --location --retry 3 "$1" -o "$2"; echo "$3  $2" | sha256sum --check -; }
fetch https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage "$stage/appimagetool" ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
fetch https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 "$stage/runtime" 2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
curl --fail --location --retry 3 https://raw.githubusercontent.com/AppImage/type2-runtime/20251108/LICENSE -o "$app/$doc/LICENSES/AppImage-runtime-LICENSE.txt"
# appimagetool adds version metadata to the root desktop file, so keep the
# manifest limited to the payload, launcher and icon that it does not modify.
(cd "$app" && find usr AppRun ez-sdr.svg -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > "$doc/SHA256SUMS")
chmod +x "$stage/appimagetool"
ARCH=x86_64 VERSION="$deb_version" APPIMAGE_EXTRACT_AND_RUN=1 "$stage/appimagetool" --runtime-file "$stage/runtime" "$app" "$out_dir/ez-sdr-${version}-x86_64.AppImage"
(cd "$out_dir" && sha256sum "ez-sdr_${deb_version}_amd64.deb" "ez-sdr-${version}-x86_64.AppImage" > "ez-sdr-${version}-linux-installers.sha256")
