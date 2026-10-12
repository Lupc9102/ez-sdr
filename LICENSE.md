# License overview

EZ-SDR has component-level license terms. Read the included texts and notices
before redistributing source or binaries:

- [`LICENSES/THIRD-PARTY-NOTICES.md`](LICENSES/THIRD-PARTY-NOTICES.md) explains
  the translated dump1090 provenance, combined binary terms, and dependency
  notices.
- [`LICENSES/GPL-2.0.txt`](LICENSES/GPL-2.0.txt) and
  [`LICENSES/GPL-3.0.txt`](LICENSES/GPL-3.0.txt) contain the GNU GPL texts used
  by the translated decoder code and combined Linux binaries.
- [`LICENSES/BSD-3-Clause-dump1090.txt`](LICENSES/BSD-3-Clause-dump1090.txt)
  preserves the BSD-3-Clause notice for original dump1090 code incorporated
  into translated modules.
- [`LICENSES/MIT.txt`](LICENSES/MIT.txt) and
  [`LICENSES/Apache-2.0.txt`](LICENSES/Apache-2.0.txt) contain the dual-license
  texts declared for independently authored project crates.

The `dump1090` crate contains code translated from dump1090-mutability, which
is GPL-2.0-or-later, and retains BSD-3-Clause-derived code. Its manifest
records `GPL-2.0-or-later AND BSD-3-Clause`. Independently authored crates
retain their `MIT OR Apache-2.0` terms. The Linux release combines GPL-2.0-or-
later components with Apache-2.0 dependencies and is offered under
GPL-3.0-or-later, as described in the notices.

Each Linux release archive includes its complete corresponding source tree,
vendored Cargo dependencies, and the license texts and notices. See
[`DISTRIBUTION.md`](DISTRIBUTION.md) for build and installation instructions.
