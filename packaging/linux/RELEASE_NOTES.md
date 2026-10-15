This update removes the unused standalone `dump1090` executable from desktop packages. Built-in ADS-B decoding remains available through the library compiled into EZ-SDR.

Linux x86-64 release for Ubuntu 24.04+, Pop!_OS 24.04+, and compatible desktops with glibc 2.39 or newer.

* **Debian/Ubuntu:** download the `.deb`, then run `sudo apt install ./ez-sdr_*_amd64.deb`. Launch **EZ-SDR** from the applications menu, or run `ez-gui`.
* **AppImage:** download the `.AppImage`, run `chmod +x ez-sdr-*-x86_64.AppImage`, then open it. If FUSE is unavailable, run it with `--appimage-extract-and-run`.
* The `.tar.gz` is a portable archive requiring the system libraries listed in `DISTRIBUTION.md`.

Verify downloads with the matching `.sha256` file and `sha256sum -c <filename>.sha256`.

Both installers include `ez-gui`, `ez-daemon`, license notices, and matching source with vendored Rust dependencies. The AppImage also bundles the RTL-SDR, USB and audio runtime libraries and their corresponding native sources. Desktop graphics drivers and an audio server come from the host.

The release pipeline installs the Debian package with apt and verifies native GUI startup from the installed application, extracted Debian package, and AppImage on X11 with software OpenGL. The attached verification report records the source commit and package hashes. Physical receiver and live audio operation require compatible hardware and are not covered by those startup checks.
