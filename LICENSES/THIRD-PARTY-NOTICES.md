# EZ-SDR distribution notices

This file identifies the licensing scopes relevant to source and binary
distributions of EZ-SDR. Preserve this file and the accompanying license texts
when redistributing the release archive.

## Combined release binaries

The `ez-gui` binary in the Linux release enables RTL-SDR support and links the
`rtlsdr_sys` Rust binding, whose published Cargo metadata declares `GPL-2.0+`
(the SPDX equivalent is `GPL-2.0-or-later`). The application also links the
`dump1090` crate. That crate contains translations of GPL-2.0-or-later code
from dump1090-mutability, including the 2.4 MHz demodulator, CRC, CPR, Mode S,
tracking, statistics, and network I/O. Some upstream files also incorporate
original BSD-3-Clause code from Salvatore Sanfilippo; `mode_ac.c` is
BSD-3-Clause. The translated Rust modules identify their upstream files in
module documentation. The crate metadata therefore records both applicable
licenses (`GPL-2.0-or-later AND BSD-3-Clause`).
Because selected project components and dependencies include Apache-2.0 code,
the combined Linux binaries are offered under GPL-3.0-or-later: the upstream
GPL-2.0-or-later grant permits that later-version choice, and GPLv3 is
compatible with Apache-2.0. The MIT OR Apache-2.0 grants remain applicable to
independently authored project components. The full GPL version 2 and version
3 texts and BSD-3-Clause text are in `GPL-2.0.txt`, `GPL-3.0.txt`, and
`BSD-3-Clause-dump1090.txt`.

The source archive shipped beside each binary archive contains the complete
source tree from the exact tagged commit used to build the binaries, all Cargo
registry dependencies vendored at the locked versions, the workspace,
packaging script, and build/install instructions. This is the corresponding
source provision for the GPL-covered portions. It is supplied with the binary
archive itself.

## Independently authored project components

The Cargo package metadata for `ez-gui`, `ez-daemon`, `ez-proto`, and
`lrpt-decode` declares `MIT OR Apache-2.0`. Those grants remain applicable to
those packages' independently authored source. The `dump1090` package metadata
records GPL-2.0-or-later AND BSD-3-Clause because it contains translated GPL
modules and BSD-derived code; that package-level expression does not revoke
any separate MIT OR Apache-2.0 grant applicable to independently authored
portions.
The license texts are included as `MIT.txt` and `Apache-2.0.txt`. Retain any
copyright and license notices present in source files.

## dump1090-mutability provenance

The translated files name their source modules, including `demod_2400.c`,
`crc.c`, `cpr.c`, `mode_ac.c`, `adaptive.c`, `stats.c`, `util.c`, `net_io.c`,
`mode_s.c`, `track.c`, and the SDR backends. Upstream dump1090-mutability says
the fork is GPL version 2 or later. The upstream source attributes its GPL
contributions to Oliver Jowett and retains BSD-3-Clause notices for code from
Salvatore Sanfilippo; its license file also credits Malcolm Robb's improvements
and SDRplay's additions. The included BSD notice preserves those upstream
terms without assigning those copyrights to EZ-SDR authors. Primary references:
<https://github.com/adsb-related-code/dump1090-mutability> and
<https://github.com/SDRplay/dump1090/blob/master/license.txt>.

## Native runtime libraries

The release binaries dynamically link platform libraries. On Ubuntu 24.04
x86_64, install the runtime packages listed in `DISTRIBUTION.md`; users may
install a compatible librtlsdr package from their distribution. Native library
licenses and notices remain those of their respective package maintainers.
`NATIVE-LIBRARIES.txt` in the binary archive records actual dynamic links and
Ubuntu builder package versions. Source projects are glibc
(<https://sourceware.org/glibc/>), GCC (<https://gcc.gnu.org/>), ALSA
(<https://github.com/alsa-project/alsa-lib>), PulseAudio
(<https://gitlab.freedesktop.org/pulseaudio/pulseaudio>), RTL-SDR
(<https://github.com/steve-m/librtlsdr>), libusb
(<https://github.com/libusb/libusb>), systemd/libudev
(<https://github.com/systemd/systemd>), and libcap
(<https://git.kernel.org/pub/scm/linux/kernel/git/morgan/libcap.git>).
