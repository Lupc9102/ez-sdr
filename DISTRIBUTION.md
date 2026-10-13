# Installing EZ-SDR releases

Release artifacts are built from a clean version tag. Each archive or installer
includes the matching application source commit, vendored Cargo dependencies,
license notices, native dependency source where applicable, and SHA-256 sums.
Verify an unpacked package with `sha256sum -c SHA256SUMS` on Linux, Git Bash,
or another system with GNU coreutils, or with `shasum -a 256 -c SHA256SUMS` on
macOS. The standalone `.sha256` file verifies the downloaded container.

## Linux x86_64

The Linux archive contains `bin/ez-gui`, `bin/ez-daemon`, and `bin/dump1090`.
It is built on Ubuntu 24.04 and uses system libraries. On Ubuntu 24.04 install:

```sh
sudo apt update
sudo apt install libasound2t64 libpulse0 librtlsdr2 libusb-1.0-0 libudev1 libcap2
```

Extract the archive and run `./bin/ez-gui`. For RTL-SDR reception, connect a
supported receiver and make it accessible to the current user. Audio can use
ALSA or a PulseAudio-compatible service such as PulseAudio or PipeWire's
PulseAudio server.

## Windows x86_64

Use either the portable ZIP or the `-setup.exe` installer. The installer adds
an EZ-SDR Start Menu shortcut and can add a desktop shortcut. The archive and
installer bundle `librtlsdr.dll`, `libusb-1.0.dll`, and every other non-system
DLL imported by the release binaries beside the executables; no separate RTL-SDR
runtime installation is needed.

On Windows, installed and portable GUI builds store settings, bookmarks, map
cache and default recordings in `%LOCALAPPDATA%\EZ-SDR`. The GUI creates this
folder before loading settings, including when launched directly from its EXE.
Missing settings/bookmarks/signal history are copied from the old launch directory
or EXE folder; existing per-user files take precedence. Old files are left in
place. Relative replay paths and custom recording locations retain their original
base; the default `./recordings` folder now lives under the per-user directory.
Settings write failures are shown in Settings and the status strip. If the
per-user folder cannot be prepared, a startup error window explains the failure.

For RTL-SDR USB reception, install the WinUSB driver for the receiver with
[Zadig](https://zadig.akeo.ie/). In Zadig, select the receiver, choose WinUSB,
and install or replace its driver. This replaces the receiver's current Windows
driver, so restore that driver with Device Manager if another application needs
it. Audio uses the Windows audio APIs and does not need a separate audio runtime.

The portable ZIP includes the matching `SOURCE-CODE.tar.gz`, `SOURCE-COMMIT.txt`,
`SOURCE/` native source archives, and `LICENSES/` notices. The installer puts the
same files under its installation directory.

The Windows release is built on a Windows 2022 runner in the MSYS2 UCRT64
environment with Rust 1.92 and the x86_64 GNU target. The packaging script fetches
the exact signed RTL-SDR and libusb packages named in `NATIVE-LIBRARIES.txt`; it
includes their matching signed source archives and the versions of the UCRT64
compiler runtimes actually installed in that build environment.

## macOS Intel x86_64

Open the DMG and drag `EZ-SDR.app` to Applications. The app bundle contains the
RTL-SDR and libusb runtime libraries in `Contents/Frameworks`; audio uses
CoreAudio and needs no separate audio runtime. The bundle also contains the
`ez-daemon` and `dump1090` command-line executables in `Contents/MacOS`.

The macOS release is an unsigned, unnotarized DMG. On first launch, macOS may
require opening the app from Finder's context menu. An Apple Developer ID,
notarization credentials, and a signing workflow are required before distributing
a notarized build.

The DMG includes `SOURCE-CODE.tar.gz`, which contains the matching EZ-SDR source,
vendored Cargo dependencies, and the exact libusb and RTL-SDR source trees used
to build the bundled libraries. License notices are in the app bundle under
`Contents/Resources/LICENSES/` and alongside the app in the DMG.

## Build from included source

The source archive targets Rust 1.92.0 and includes the locked Cargo registry
sources. Platform build details and native source versions are recorded in
`NATIVE-LIBRARIES.txt`. Each platform's source archive and vendor dependencies
are generated from the same tagged commit as its binaries. See `LICENSES/` for
license texts and component provenance.

## 978 MHz UAT

All desktop packages include the Rust native UAT decoder. Native IQ mode needs
no separately installed decoder; the selected capture must be centered on
978 MHz and run at 2,083,333 complex samples/second. The optional dump978-fa
JSON source mode requires a separately installed dump978-fa and its compatible
SDR driver. See the UAT section in the README for setup details.
