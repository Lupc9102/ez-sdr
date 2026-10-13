#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

version="${RELEASE_VERSION:-$(git describe --tags --exact-match 2>/dev/null || true)}"
if [[ -z "$version" ]]; then
  echo "Set RELEASE_VERSION or run from an exact release tag." >&2
  exit 2
fi
if [[ "${MSYSTEM:-}" != UCRT64 ]]; then
  echo "Run this script in the MSYS2 UCRT64 environment." >&2
  exit 2
fi

# setup-msys2's PATH filtering can hide GitHub's rustup installation from an
# MSYS shell. Resolve the Windows profile explicitly so the pinned Rust tools
# installed by dtolnay/rust-toolchain remain available in UCRT64.
if [[ -n "${CARGO_HOME:-}" ]]; then
  cargo_bin="$(cygpath -u "$CARGO_HOME")/bin"
elif [[ -n "${USERPROFILE:-}" ]]; then
  cargo_bin="$(cygpath -u "$USERPROFILE")/.cargo/bin"
else
  echo "Cannot locate the Windows user profile containing the Rust toolchain." >&2
  exit 2
fi
export PATH="$cargo_bin:$PATH"
command -v cargo >/dev/null || { echo "cargo is missing from the UCRT64 PATH." >&2; exit 2; }
command -v rustup >/dev/null || { echo "rustup is missing from the UCRT64 PATH." >&2; exit 2; }
command -v x86_64-w64-mingw32-gcc >/dev/null || { echo "The UCRT64 MinGW linker is unavailable." >&2; exit 2; }

out_dir="${1:-dist}"
mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd)"
stage="$(mktemp -d)"
source_root="$(mktemp -d)"
trap 'rm -rf "$stage" "$source_root"' EXIT

name="ez-sdr-${version}-windows-x86_64"
package_dir="$stage/$name"
mkdir -p "$package_dir/bin" "$package_dir/LICENSES" "$package_dir/SOURCE"

release_inputs=(
  Cargo.toml Cargo.lock deny.toml DISTRIBUTION.md LICENSE.md README.md rust-toolchain.toml
  .github/workflows/release.yml scripts/package-linux-release.sh scripts/package-windows-release.sh
  scripts/ez-sdr-installer.iss scripts/package-macos-release.sh
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

# Freeze the redistributable native runtime versions. Update these pins and
# review the upstream source/license snapshots deliberately when dependencies
# need to move.
readonly rtl_sdr_version=2.0.3-1
readonly libusb_version=1.0.30-1
readonly msys2_mingw_repo=https://repo.msys2.org/mingw/ucrt64
native_packages="$stage/native-packages"
mkdir -p "$native_packages"
download_signed_package() {
  local name="$1" version="$2" filename="$name-$version-any.pkg.tar.zst"
  curl --fail --location --retry 3 "$msys2_mingw_repo/$filename" \
    -o "$native_packages/$filename"
  curl --fail --location --retry 3 "$msys2_mingw_repo/$filename.sig" \
    -o "$native_packages/$filename.sig"
  # Fail before installing if the detached signature is invalid or untrusted.
  pacman-key --verify "$native_packages/$filename.sig" "$native_packages/$filename"
}

# Install the exact signed RTL-SDR and libusb runtime artifacts whose source
# snapshots and licenses are shipped below. This avoids coupling release
# success to whatever versions happen to be in the rolling package database.
download_signed_package mingw-w64-ucrt-x86_64-rtl-sdr "$rtl_sdr_version"
download_signed_package mingw-w64-ucrt-x86_64-libusb "$libusb_version"
pacman -U --noconfirm \
  "$native_packages/mingw-w64-ucrt-x86_64-libusb-$libusb_version-any.pkg.tar.zst" \
  "$native_packages/mingw-w64-ucrt-x86_64-rtl-sdr-$rtl_sdr_version-any.pkg.tar.zst"

check_package_version() {
  local package="$1" expected="$2" actual
  actual="$(pacman -Q "$package" | awk '{print $2}')"
  if [[ "$actual" != "$expected" ]]; then
    echo "Expected $package $expected, found $actual; update the native release pins and review their sources/licenses." >&2
    exit 1
  fi
}
check_package_version mingw-w64-ucrt-x86_64-rtl-sdr "$rtl_sdr_version"
check_package_version mingw-w64-ucrt-x86_64-libusb "$libusb_version"

rustup target add x86_64-pc-windows-gnu
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
# Cargo and build scripts are native Windows processes, so pass pkgconf native
# Windows paths. Do not inherit a host's pkg-config search paths: those can
# silently resolve Linux/MSYS packages while compiling the GNU Windows target.
pkgconf_exe="$(command -v pkgconf || command -v pkg-config || true)"
if [[ -z "$pkgconf_exe" ]]; then
  echo "MSYS2 UCRT64 pkgconf is required to locate the pinned native libraries." >&2
  exit 2
fi
ucrt_pkgconfig_dir="$(cygpath -w /ucrt64/lib/pkgconfig)"
ucrt_share_pkgconfig="$(cygpath -w /ucrt64/share/pkgconfig)"
# pkg-config-rs prefers target-specific variables over the plain names. Clear
# those too so a runner-level cross sysroot or host package path cannot win.
unset \
  PKG_CONFIG_x86_64_pc_windows_gnu \
  PKG_CONFIG_PATH_x86_64_pc_windows_gnu PKG_CONFIG_LIBDIR_x86_64_pc_windows_gnu PKG_CONFIG_SYSROOT_DIR_x86_64_pc_windows_gnu \
  PKG_CONFIG_ALLOW_CROSS_x86_64_pc_windows_gnu \
  TARGET_PKG_CONFIG_PATH TARGET_PKG_CONFIG_LIBDIR TARGET_PKG_CONFIG_SYSROOT_DIR \
  HOST_PKG_CONFIG_PATH HOST_PKG_CONFIG_LIBDIR HOST_PKG_CONFIG_SYSROOT_DIR \
  PKG_CONFIG_PATH_FOR_TARGET \
  PKG_CONFIG_SYSROOT_DIR
export PKG_CONFIG="$(cygpath -w "$pkgconf_exe")"
export PKG_CONFIG_PATH="$ucrt_pkgconfig_dir"
export PKG_CONFIG_LIBDIR="$ucrt_pkgconfig_dir;$ucrt_share_pkgconfig"
export PKG_CONFIG_ALLOW_CROSS=1
for module in libusb-1.0 librtlsdr; do
  pkgconfig_libdir="$("$PKG_CONFIG" --variable=libdir "$module")"
  pkgconfig_libdir_unix="$(cygpath -u "$pkgconfig_libdir")"
  if [[ "$pkgconfig_libdir_unix" != /ucrt64/lib ]]; then
    echo "pkg-config resolved $module outside the pinned UCRT64 prefix: $pkgconfig_libdir" >&2
    exit 1
  fi
done
cargo build --workspace --locked --release --target x86_64-pc-windows-gnu \
  --features ez-daemon/rtlsdr,dump1090/rtlsdr

for binary in ez-gui.exe ez-daemon.exe dump1090.exe; do
  source="target/x86_64-pc-windows-gnu/release/$binary"
  [[ -s "$source" ]] || { echo "Missing release binary: $source" >&2; exit 1; }
  install -m 0755 "$source" "$package_dir/bin/$binary"
done

# Include the UCRT64 runtime DLLs pulled in by the RTL-SDR and GNU toolchain
# builds. Fail if a dependency is absent rather than emitting an unusable ZIP.
# Recursively collect non-system DLL imports reported by the PE linker.
install -m 0644 /ucrt64/bin/librtlsdr.dll "$package_dir/bin/librtlsdr.dll"
queue=("$package_dir/bin/ez-gui.exe" "$package_dir/bin/ez-daemon.exe" "$package_dir/bin/dump1090.exe" /ucrt64/bin/librtlsdr.dll)
declare -A copied
while ((${#queue[@]})); do
  binary="${queue[0]}"
  queue=("${queue[@]:1}")
  while IFS= read -r dll; do
    [[ -n "$dll" ]] || continue
    # Windows system DLLs are supplied by supported Windows installations.
    case "${dll,,}" in
      api-ms-win-*.dll|ext-ms-win-*.dll|kernel32.dll|user32.dll|gdi32.dll|winmm.dll|\
      advapi32.dll|shell32.dll|ole32.dll|oleaut32.dll|comdlg32.dll|comctl32.dll|\
      ws2_32.dll|bcrypt.dll|secur32.dll|ntdll.dll|setupapi.dll|cfgmgr32.dll|\
      msvcrt.dll|ucrtbase.dll|vcruntime140.dll) continue ;;
    esac
    [[ -z "${copied[${dll,,}]:-}" ]] || continue
    source="/ucrt64/bin/$dll"
    if [[ ! -s "$source" ]]; then
      if [[ -e "/c/Windows/System32/$dll" || -e "/c/Windows/SysWOW64/$dll" ]]; then
        continue
      fi
      echo "Missing non-system DLL dependency $dll required by $binary" >&2
      exit 1
    fi
    install -m 0644 "$source" "$package_dir/bin/$dll"
    copied[${dll,,}]=1
    queue+=("$source")
  done < <(objdump -p "$binary" | awk '/DLL Name:/ {print $3}')
done

install -m 0644 DISTRIBUTION.md LICENSE.md "$package_dir/"
install -m 0644 LICENSES/*.txt LICENSES/THIRD-PARTY-NOTICES.md "$package_dir/LICENSES/"

# Preserve exact MSYS2 package source snapshots for the native DLLs. These are
# the source provisions corresponding to the native binaries in bin/.
for pkg in \
  mingw-w64-ucrt-x86_64-rtl-sdr \
  mingw-w64-ucrt-x86_64-libusb \
  mingw-w64-ucrt-x86_64-gcc-libs \
  mingw-w64-ucrt-x86_64-cc-libs \
  mingw-w64-ucrt-x86_64-libgcc \
  mingw-w64-ucrt-x86_64-libstdc++ \
  mingw-w64-ucrt-x86_64-libwinpthread; do
  if pacman -Q "$pkg" >/dev/null 2>&1; then
    pkg_version="$(pacman -Q "$pkg" | awk '{print $2}')"
    base="${pkg#mingw-w64-ucrt-x86_64-}"
    case "$base" in
      gcc-libs|cc-libs|libgcc|libstdc++) base=gcc ;;
      libwinpthread) base=winpthreads ;;
    esac
    url="https://repo.msys2.org/mingw/sources/mingw-w64-${base}-${pkg_version}.src.tar.zst"
    source_archive="$package_dir/SOURCE/${base}-${pkg_version}.src.tar.zst"
    if [[ ! -s "$source_archive" ]]; then
      curl --fail --location --retry 3 "$url" -o "$source_archive"
      curl --fail --location --retry 3 "$url.sig" -o "$source_archive.sig"
      pacman-key --verify "$source_archive.sig" "$source_archive"
    fi
  fi
done
rtl_sdr_sources=("$package_dir"/SOURCE/rtl-sdr-*.src.tar.zst)
libusb_sources=("$package_dir"/SOURCE/libusb-*.src.tar.zst)
[[ ${#rtl_sdr_sources[@]} -eq 1 && -s "${rtl_sdr_sources[0]}" ]] || {
  echo "Missing or ambiguous matching rtl-sdr source archive." >&2
  exit 1
}
[[ ${#libusb_sources[@]} -eq 1 && -s "${libusb_sources[0]}" ]] || {
  echo "Missing or ambiguous matching libusb source archive." >&2
  exit 1
}

for license in rtl-sdr libusb; do
  source="/ucrt64/share/licenses/$license/COPYING"
  [[ -s "$source" ]] || { echo "Missing native license: $source" >&2; exit 1; }
  install -m 0644 "$source" "$package_dir/LICENSES/$license-COPYING.txt"
done
gcc_runtime_license_count=0
for license_dir in gcc libgcc gcc-libs; do
  for notice in "/ucrt64/share/licenses/$license_dir"/*; do
    [[ -f "$notice" ]] || continue
    install -m 0644 "$notice" "$package_dir/LICENSES/gcc-$license_dir-$(basename "$notice")"
    gcc_runtime_license_count=$((gcc_runtime_license_count + 1))
  done
done
((gcc_runtime_license_count > 0)) || {
  echo "Missing GCC runtime license notices under /ucrt64/share/licenses." >&2
  exit 1
}
install -m 0644 /ucrt64/share/licenses/libwinpthread/COPYING \
  "$package_dir/LICENSES/winpthreads-COPYING.txt"

# Vendor Cargo dependencies from the exact source commit used for the build.
git archive --format=tar HEAD | tar -xf - -C "$source_root"
mkdir -p "$source_root/.cargo"
(cd "$source_root" && cargo vendor --locked vendor > .cargo/config.toml)
tar -czf "$package_dir/SOURCE-CODE.tar.gz" -C "$source_root" .
git rev-parse HEAD > "$package_dir/SOURCE-COMMIT.txt"

{
  echo "Target: x86_64-pc-windows-gnu (MSYS2 UCRT64)"
  echo "MSYS2 runtime package versions used for bundled DLLs:"
  for pkg in \
    mingw-w64-ucrt-x86_64-rtl-sdr \
    mingw-w64-ucrt-x86_64-libusb \
    mingw-w64-ucrt-x86_64-gcc-libs \
    mingw-w64-ucrt-x86_64-cc-libs \
    mingw-w64-ucrt-x86_64-libgcc \
    mingw-w64-ucrt-x86_64-libstdc++ \
    mingw-w64-ucrt-x86_64-libwinpthread; do
    pacman -Q "$pkg" 2>/dev/null || true
  done
  echo
  echo "The installer copies the bundled DLLs beside the executables, where Windows searches for dependent DLLs."
  echo "Signed runtime pins: rtl-sdr=$rtl_sdr_version libusb=$libusb_version"
  echo "GCC and winpthreads runtime package versions and matching source snapshots are listed above and included in SOURCE/."
  echo "RTL-SDR USB access on Windows requires replacing the receiver's vendor driver with WinUSB using Zadig."
  echo "The bundled non-system DLLs are:"
  find "$package_dir/bin" -maxdepth 1 -iname '*.dll' -printf '  %f\n' | sort
  echo "The matching MSYS2 source snapshots for RTL-SDR, libusb, GCC runtime, and winpthreads are in SOURCE/."
} > "$package_dir/NATIVE-LIBRARIES.txt"

cp scripts/ez-sdr-installer.iss "$stage/installer.iss"
"/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
  "/DAppVersion=${version#v}" "/DSourceDir=$(cygpath -w "$package_dir")" "/DOutputDir=$(cygpath -w "$out_dir")" \
  "$stage/installer.iss"

(cd "$package_dir" && sha256sum bin/* SOURCE/* SOURCE-CODE.tar.gz SOURCE-COMMIT.txt DISTRIBUTION.md LICENSE.md NATIVE-LIBRARIES.txt LICENSES/* > SHA256SUMS)
(cd "$stage" && 7z a -tzip "$out_dir/$name.zip" "$name") >/dev/null
(cd "$out_dir" && sha256sum "$name.zip" "$name-setup.exe" > "$name.sha256")
echo "$out_dir/$name.zip"
echo "$out_dir/$name-setup.exe"
