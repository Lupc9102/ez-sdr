#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
version="${RELEASE_VERSION:-$(git describe --tags --exact-match 2>/dev/null || true)}"
if [[ -z "$version" ]]; then
  echo "Set RELEASE_VERSION or run from an exact release tag." >&2
  exit 2
fi
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != x86_64 ]]; then
  echo "Run this script on an Intel macOS runner." >&2
  exit 2
fi

# Native libraries are built from fixed upstream source tags so the app bundle
# and its matching source provision remain reproducible across runner updates.
RTL_SDR_VERSION=2.0.3
RTL_SDR_SHA256=23dec530f40456a8396e7c897f313859877e18b4c6432c6d323645871e351f6f
LIBUSB_VERSION=1.0.30
LIBUSB_SHA256=fea36f34f9156400209595e300840767ab1a385ede1dc7ee893015aea9c6dbaf
out_dir="${1:-dist}"
mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd)"
stage="$(mktemp -d)"
source_root="$(mktemp -d)"
trap 'rm -rf "$stage" "$source_root"' EXIT

name="ez-sdr-${version}-macos-x86_64"
app="$stage/$name/EZ-SDR.app"
contents="$app/Contents"
mkdir -p "$contents/MacOS" "$contents/Frameworks" "$contents/Resources/LICENSES" "$stage/$name/LICENSES"

release_inputs=(
  Cargo.toml Cargo.lock deny.toml DISTRIBUTION.md LICENSE.md README.md rust-toolchain.toml
  .github/workflows/release.yml scripts/package-linux-release.sh scripts/package-windows-release.sh
  scripts/package-macos-release.sh scripts/ez-sdr-installer.iss
  dump1090 ez-gui ez-daemon ez-proto lrpt-decode uat-decode
  LICENSES/GPL-2.0.txt LICENSES/GPL-3.0.txt LICENSES/MIT.txt
  LICENSES/BSD-2-Clause-FlightAware.txt LICENSES/LGPL-2.1-or-later.txt
  LICENSES/Apache-2.0.txt LICENSES/BSD-3-Clause-dump1090.txt LICENSES/THIRD-PARTY-NOTICES.md
)
if [[ -n "$(git status --porcelain --untracked-files=all -- "${release_inputs[@]}")" ]]; then
  echo "Release inputs differ from HEAD; commit the source used for this build first." >&2
  exit 1
fi
git ls-files --error-unmatch "${release_inputs[@]}" >/dev/null

native="$stage/native"
mkdir -p "$native/src" "$native/prefix"
curl --fail --location --retry 3 "https://github.com/libusb/libusb/releases/download/v${LIBUSB_VERSION}/libusb-${LIBUSB_VERSION}.tar.bz2" -o "$native/libusb.tar.bz2"
curl --fail --location --retry 3 "https://github.com/osmocom/rtl-sdr/archive/v${RTL_SDR_VERSION}/rtl-sdr-${RTL_SDR_VERSION}.tar.gz" -o "$native/rtl-sdr.tar.gz"
echo "$LIBUSB_SHA256  $native/libusb.tar.bz2" | shasum -a 256 -c -
tar -xjf "$native/libusb.tar.bz2" -C "$native/src"
echo "$RTL_SDR_SHA256  $native/rtl-sdr.tar.gz" | shasum -a 256 -c -
tar -xzf "$native/rtl-sdr.tar.gz" -C "$native/src"
libusb_src="$native/src/libusb-${LIBUSB_VERSION}"
rtlsdr_src="$native/src/rtl-sdr-${RTL_SDR_VERSION}"
prefix="$native/prefix"

(cd "$libusb_src" && ./configure --prefix="$prefix" --enable-shared --disable-static)
make -C "$libusb_src" -j4
make -C "$libusb_src" install

cat > "$native/libusb-1.0.pc" <<PC
prefix=$prefix
libdir=\${prefix}/lib
includedir=\${prefix}/include
Name: libusb-1.0
Description: USB device access library
Version: $LIBUSB_VERSION
Libs: -L\${libdir} -lusb-1.0
Cflags: -I\${includedir}/libusb-1.0
PC
export PKG_CONFIG_PATH="$native:$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export PKG_CONFIG_LIBDIR="$PKG_CONFIG_PATH"
export MACOSX_DEPLOYMENT_TARGET=12.0
cmake -S "$rtlsdr_src" -B "$native/rtlsdr-build" -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$prefix" -DLIBUSB_INCLUDE_DIRS="$prefix/include/libusb-1.0" \
  -DLIBUSB_LIBRARIES="$prefix/lib/libusb-1.0.dylib"
cmake --build "$native/rtlsdr-build" --config Release --parallel 4
cmake --install "$native/rtlsdr-build" --config Release

export PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$PKG_CONFIG_PATH"
rustup target add x86_64-apple-darwin
cargo build --locked --release -p ez-gui -p ez-daemon --target x86_64-apple-darwin \
  --features ez-daemon/rtlsdr

for binary in ez-gui ez-daemon; do
  source="target/x86_64-apple-darwin/release/$binary"
  [[ -s "$source" ]] || { echo "Missing release binary: $source" >&2; exit 1; }
  install -m 0755 "$source" "$contents/MacOS/$binary"
done

for library in "$prefix"/lib/librtlsdr*.dylib "$prefix"/lib/libusb-1.0*.dylib; do
  [[ -s "$library" ]] || { echo "Missing native library: $library" >&2; exit 1; }
  install -m 0755 "$library" "$contents/Frameworks/$(basename "$library")"
done

# Normalize library IDs and all load commands to the app's Frameworks folder.
for library in "$contents"/Frameworks/*.dylib; do
  base="$(basename "$library")"
  install_name_tool -id "@rpath/$base" "$library"
done
for library in "$contents"/Frameworks/*.dylib "$contents"/MacOS/*; do
  while IFS= read -r dependency; do
    case "$dependency" in
      *librtlsdr*.dylib|*libusb-1.0*.dylib)
        target="@executable_path/../Frameworks/$(basename "$dependency")"
        # A dylib resolves its own dependencies relative to its loader.
        [[ "$library" == *.dylib ]] && target="@loader_path/$(basename "$dependency")"
        install_name_tool -change "$dependency" "$target" "$library"
        ;;
    esac
  done < <(otool -L "$library" | sed -n 's/^[[:space:]]*\([^[:space:]]*\.dylib\).*/\1/p')
  if [[ "$library" != *.dylib ]]; then
    if ! otool -l "$library" | grep -Fq '@executable_path/../Frameworks'; then
      install_name_tool -add_rpath '@executable_path/../Frameworks' "$library"
    fi
  fi
done

cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>ez-gui</string>
  <key>CFBundleIdentifier</key><string>org.ezsdr.EZSDR</string>
  <key>CFBundleName</key><string>EZ-SDR</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version#v}</string>
  <key>CFBundleVersion</key><string>${version#v}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
PLIST
install -m 0644 DISTRIBUTION.md LICENSE.md "$stage/$name/"
install -m 0644 LICENSES/*.txt LICENSES/THIRD-PARTY-NOTICES.md "$contents/Resources/LICENSES/"
install -m 0644 LICENSES/*.txt LICENSES/THIRD-PARTY-NOTICES.md "$stage/$name/LICENSES/"
install -m 0644 "$rtlsdr_src/COPYING" "$stage/$name/LICENSES/rtl-sdr-COPYING.txt"
install -m 0644 "$libusb_src/COPYING" "$stage/$name/LICENSES/libusb-COPYING.txt"
install -m 0644 "$rtlsdr_src/COPYING" "$contents/Resources/LICENSES/rtl-sdr-COPYING.txt"
install -m 0644 "$libusb_src/COPYING" "$contents/Resources/LICENSES/libusb-COPYING.txt"

# The source release includes all Rust registry sources plus the exact native
# upstream source trees used above, not only links to them.
git archive --format=tar HEAD | tar -xf - -C "$source_root"
mkdir -p "$source_root/.cargo" "$source_root/native-source/libusb" "$source_root/native-source/rtl-sdr"
(cd "$source_root" && cargo vendor --locked vendor > .cargo/config.toml)
cp -R "$libusb_src/." "$source_root/native-source/libusb/"
cp -R "$rtlsdr_src/." "$source_root/native-source/rtl-sdr/"
tar -czf "$stage/$name/SOURCE-CODE.tar.gz" -C "$source_root" .
git rev-parse HEAD > "$stage/$name/SOURCE-COMMIT.txt"
{
  echo "Target: x86_64-apple-darwin; minimum macOS 12.0"
  echo "RTL-SDR: osmocom/rtl-sdr tag v$RTL_SDR_VERSION (SHA-256 $RTL_SDR_SHA256; GPL-2.0-or-later)"
  echo "USB runtime: libusb tag v$LIBUSB_VERSION (SHA-256 $LIBUSB_SHA256; LGPL-2.1-or-later)"
  echo "Bundled native libraries:"
  find "$contents/Frameworks" -maxdepth 1 -name "*.dylib" -exec basename {} \; | sort | sed 's/^/  /'
  echo "Both native libraries use app-relative load commands."
  echo "Audio output uses macOS CoreAudio through CPAL; no third-party audio runtime is required."
  echo "This DMG is unsigned and not notarized."
} > "$stage/$name/NATIVE-LIBRARIES.txt"

(cd "$stage/$name" && shasum -a 256 EZ-SDR.app/Contents/MacOS/* EZ-SDR.app/Contents/Frameworks/* SOURCE-CODE.tar.gz SOURCE-COMMIT.txt DISTRIBUTION.md LICENSE.md NATIVE-LIBRARIES.txt LICENSES/* > SHA256SUMS)
dmg_stage="$stage/dmg"
mkdir -p "$dmg_stage"
cp -R "$app" "$dmg_stage/"
cp "$stage/$name/DISTRIBUTION.md" "$stage/$name/LICENSE.md" "$stage/$name/SOURCE-CODE.tar.gz" "$stage/$name/SOURCE-COMMIT.txt" "$stage/$name/NATIVE-LIBRARIES.txt" "$stage/$name/SHA256SUMS" "$dmg_stage/"
cp -R "$stage/$name/LICENSES" "$dmg_stage/"
hdiutil create -volname EZ-SDR -srcfolder "$dmg_stage" -ov -format UDZO "$out_dir/$name.dmg"
(cd "$out_dir" && shasum -a 256 "$name.dmg" > "$name.dmg.sha256")
echo "$out_dir/$name.dmg"
