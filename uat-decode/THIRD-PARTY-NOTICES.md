# Third-party notices

The protocol constants and receiver/payload behavior in this crate are based on
FlightAware's dump978 implementation. Its demodulator includes code by Oliver
Jowett (2015) and FlightAware LLC (2019); ADS-B field interpretation and UAT
frame/FEC parameters are from FlightAware LLC (2019). Their BSD 2-Clause terms
are in `LICENSES/BSD-2-Clause-FlightAware.txt`.

`src/fec.rs` is a Rust translation of the general-purpose Reed-Solomon decoder
algorithm by Phil Karn (KA9Q), as included in FlightAware's dump978 `libs/fec`.
The upstream code is Copyright 2002–2004 Phil Karn and is made available under
the GNU Lesser General Public License, version 2.1 or later. The translation
uses dump978's UAT parameters and adapts the decoder to return corrected
shortened frames. The full license is in `LICENSES/LGPL-2.1-or-later.txt`.

Upstream reference (pinned):
<https://github.com/flightaware/dump978/tree/f40aa9a0d2d0a0067c0c628b44ba316f7049b8ce>

The implementation does not link or invoke dump978, libfec, or another runtime
program. The corresponding translated source is in this crate's `src/` tree.
