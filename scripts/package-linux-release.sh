#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

version="${RELEASE_VERSION:-$(git describe --tags --exact-match 2>/dev/null || true)}"
if [[ -z "$version" ]]; then
  echo "Set RELEASE_VERSION or run from an exact release tag." >&2
  exit 2
fi

out_dir="${1:-dist}"
mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd)"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

name="ez-sdr-${version}-linux-x86_64"
package_dir="$stage/$name"
mkdir -p "$package_dir/bin" "$package_dir/LICENSES"

release_inputs=(
  Cargo.toml Cargo.lock deny.toml DISTRIBUTION.md LICENSE.md README.md rust-toolchain.toml
  .github/workflows/release.yml scripts/package-linux-release.sh scripts/package-windows-release.sh
  scripts/package-macos-release.sh scripts/ez-sdr-installer.iss
  scripts/package-linux-installers.sh scripts/verify-linux-packages.py packaging/linux
  dump1090 ez-gui ez-daemon ez-proto lrpt-decode uat-decode
  LICENSES/GPL-2.0.txt LICENSES/GPL-3.0.txt LICENSES/MIT.txt
  LICENSES/BSD-2-Clause-FlightAware.txt LICENSES/LGPL-2.1-or-later.txt
  LICENSES/Apache-2.0.txt LICENSES/BSD-3-Clause-dump1090.txt
  LICENSES/THIRD-PARTY-NOTICES.md
)
if [[ -n "$(git status --porcelain --untracked-files=all -- "${release_inputs[@]}")" ]]; then
  echo "Release inputs differ from HEAD; commit the source used for this build first." >&2
  exit 1
fi
git ls-files --error-unmatch "${release_inputs[@]}" >/dev/null

# Build after the source check: earlier headless tests may have replaced the
# executables in target/release with a different feature configuration.
cargo build --workspace --locked --release --features ez-daemon/rtlsdr,dump1090/rtlsdr

for binary in ez-gui ez-daemon dump1090; do
  source="target/release/$binary"
  if [[ ! -x "$source" ]]; then
    echo "Missing release binary: $source" >&2
    exit 1
  fi
  if ldd "$source" | rg -q 'not found'; then
    echo "Unresolved shared library in $source" >&2
    ldd "$source" >&2
    exit 1
  fi
  install -m 0755 "$source" "$package_dir/bin/$binary"
done

install -m 0644 DISTRIBUTION.md LICENSE.md "$package_dir/"
install -m 0644 LICENSES/*.txt LICENSES/THIRD-PARTY-NOTICES.md "$package_dir/LICENSES/"

source_root="$(mktemp -d)"
trap 'rm -rf "$stage" "$source_root"' EXIT
git archive --format=tar HEAD | tar -xf - -C "$source_root"
mkdir -p "$source_root/.cargo"
(cd "$source_root" && cargo vendor --locked vendor > .cargo/config.toml)
tar -czf "$package_dir/SOURCE-CODE.tar.gz" -C "$source_root" .
git rev-parse HEAD > "$package_dir/SOURCE-COMMIT.txt"

{
  echo "Dynamic dependencies for release binaries (ldd):"
  for binary in "$package_dir/bin"/*; do
    echo
    echo "[$(basename "$binary")]"
    ldd "$binary"
  done
  echo
  echo "Ubuntu 24.04 runtime/build package versions on the builder:"
  dpkg-query -W -f='${binary:Package}\t${Version}\n' \
    libasound2t64 libpulse0 librtlsdr2 librtlsdr-dev libusb-1.0-0 libudev1 libcap2 libc6 libgcc-s1 \
    2>/dev/null || true
  echo "The GUI may load libpulse at runtime through CPAL even when ldd does not list it."
} > "$package_dir/NATIVE-LIBRARIES.txt"

(cd "$package_dir" && sha256sum bin/* SOURCE-CODE.tar.gz SOURCE-COMMIT.txt DISTRIBUTION.md LICENSE.md NATIVE-LIBRARIES.txt LICENSES/* > SHA256SUMS)
tar -czf "$out_dir/$name.tar.gz" -C "$stage" "$name"
(cd "$out_dir" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
scripts/package-linux-installers.sh "$package_dir" "$out_dir" "$version"
echo "$out_dir/$name.tar.gz"
echo "$out_dir/$name.tar.gz.sha256"
