# Linux release installation

The release workflow builds x86_64 Linux binaries with the RTL-SDR backend
enabled. Each archive contains `bin/ez-gui`, `bin/ez-daemon`, and
`bin/dump1090`, this guide, license notices, and `SOURCE-CODE.tar.gz` from the
same tagged commit. The source archive includes the locked Cargo dependencies
as vendored source. Verify the archive with `sha256sum -c SHA256SUMS` before
installation.

## Ubuntu 24.04 runtime

Install the native runtime libraries:

```sh
sudo apt update
sudo apt install libasound2t64 libpulse0 librtlsdr2 libusb-1.0-0 libudev1 libcap2
```

Extract the archive and run a binary from its `bin/` directory. For example:

```sh
./bin/ez-gui
```

The RTL-SDR backend requires a supported USB receiver and device permissions.
Without hardware, `ez-gui` can still be used for its other available workflows.
Audio output can use ALSA (`libasound2`) or PulseAudio (`libpulse0`). The GUI
enables CPAL's PulseAudio host and prefers it when available; a
PulseAudio-compatible service such as PulseAudio or PipeWire's PulseAudio
server should be running for that output path.

## Build from the included source

The included source tree targets Rust 1.92.0. Cargo is configured to use the
vendored dependencies in `vendor/`. On Ubuntu 24.04, install the build
dependencies and compile the same release features offline with:

```sh
sudo apt update
sudo apt install build-essential pkg-config libasound2-dev libpulse-dev librtlsdr-dev
rustup toolchain install 1.92.0
cargo +1.92.0 build --workspace --locked --offline --release --features ez-daemon/rtlsdr,dump1090/rtlsdr
```

The resulting executables are in `target/release/`. The release archive's
`SOURCE-CODE.tar.gz` was generated from the exact source commit for that
release. License and component provenance are in `LICENSES/`.

## Platform scope

This package is for x86_64 Linux. The release workflow does not claim to
provide Windows or macOS binaries, a portable installer, or bundled native
system libraries. The documented Ubuntu 24.04 package names are intended for
the Ubuntu 24.04 release runner and compatible systems; other distributions
may use different package names. Install the distro packages matching the
binary's dynamic library requirements.
