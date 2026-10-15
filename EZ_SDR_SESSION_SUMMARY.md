# EZ-SDR Audit Remediation Session Summary

## Session identity

- Date: 2026-09-20
- Harness/model: Codex / GPT-6 / `functions.exec`
- Tracker: `TASK_TRACKER.md`
- Reconciled audits: `FUNCTIONALITY_WEB_AUDIT_REPORT.md`, `ADSB_DOCS_AUDIT.md`, `LRPT_DOCS_AUDIT.md`, and `FREQUENCY_WEB_SOURCES_AUDIT.md`

## Work completed

### ADS-B / Mode S

- Preserved magnitude overlap across GUI and daemon IQ block boundaries and corrected cumulative 2.4 MSPS timestamp scaling.
- Corrected short-message CRC/error-correction ordering, DF11 syndrome rejection, and shared altitude/Gillham handling.
- Removed phantom aircraft created from Mode A/C replies.
- Added global-longitude normalization, established-track local CPR, reference-based surface CPR, and antimeridian regressions.
- Added ICAO-filter TTL and insertion-driven rotation.
- Decoded TC19 vertical rate and propagated it through daemon/web telemetry.
- Kept product claims scoped to 1090ES; 978 MHz UAT is not represented as supported.

### Meteor LRPT

- Added CCSDS K=7, rate-1/2 Viterbi decoding with 171/133 octal generators and the published coded ASM.
- Implemented native CCSDS RS(255,223) with polynomial `0x187`, first root 112, primitive element 11, and four-way interleaving. The test suite includes an independent libfec parity vector.
- Corrected receive order to Viterbi, optional NRZ-M, then CADU sync and added eight phase/axis hypotheses.
- Added OQPSK half-symbol Q reconstruction.
- Corrected the VCDU insert-zone/M-PDU offsets and added a realistic packet-layout regression.
- Added MSU-MR Huffman, quantization, IDCT, MCU, and 1568-pixel line reconstruction.
- Updated active Meteor-M2-3/M2-4 desktop and browser workflows. Defaults use 80 ksym/s; the decoder API remains configurable for 72 ksym/s recordings.

### Satellite propagation and Doppler

- Replaced the fabricated mean-motion sine-wave model with the `sgp4` crate and complete TLE parsing.
- Added current, dated Celestrak fallback elements for Meteor-M2-3, Meteor-M2-4, and ISS.
- Added Satellite-panel import for current 2LE/3LE files, with atomic parsing and NORAD-ID matching to the built-in catalog.
- Converted TEME position/velocity into an Earth-fixed frame and included Earth rotation in range-rate calculations.
- Added WGS84 observer ECEF and ENU azimuth/elevation/slant-range geometry, sub-satellite coordinates, refined AOS/LOS crossings, pass maxima, and ground tracks.
- Computed Doppler from observer-relative range rate.
- Kept the LRPT receiver at nominal center frequency during a pass, avoiding repeated retunes while still displaying Doppler.

### DSP, hardware, frequency, and web claims

- Added complex channel filtering before GUI AM and FM nonlinear demodulation/decimation and enabled AM carrier/DC blocking.
- Made GUI and daemon USB/LSB sideband selective; the daemon uses a complex FIR and has an opposite-sideband rejection regression.
- Mapped HackRF gain across RF amp, LNA, and VGA stages with error propagation and cached-state rollback.
- Added selectable ITU Region 1/2/3 amateur overlays; corrected Region 2 1.25 m, regional 80 m/40 m/70 cm, maritime MF spots, and GPS L1/L2C spans.
- Split ISS Voice/SSTV 145.800 MHz and APRS 145.825 MHz entries.
- Clarified mono WFM/no RDS, external-BFO CW scope, manual TLE updates, and 80/72 ksym/s LRPT behavior.
- Removed plaintext IP geolocation and the local DuckDuckGo Lite scraper. Search now requests the selected provider's native tool with a disclosure.
- Corrected Planespotters parsing/image URLs and retained OSM attribution.
- Reconciled each detailed audit with a current-tree resolution table while preserving the original research as historical evidence.

## Validation evidence

- `cargo fmt --all`: passed.
- `cargo test -p lrpt-decode`: 65 passed.
- `cargo test -p dump1090`: 246 passed in the library target and 246 passed in the binary target.
- `cargo test -p ez-gui --lib`: 476 passed with loopback sockets enabled. The same suite's seven socket tests were confirmed to be sandbox-only failures before the permitted rerun.
- `cargo test --workspace`: passed, including integration and documentation tests.
- `cargo check --workspace --all-features`: passed. Expected warnings report missing host pkg-config packages for optional HackRF and SoapySDR backends.
- `ez-web`: all 3 Node test suites passed and the Vite production build passed.
- `git diff --check`: passed.

## Remaining evidence limits

- No physical SDR was available, so real device behavior, RF sensitivity, antennas, and host drivers remain unverified.
- No independently sourced off-air Meteor IQ capture with a known image is checked in. Protocol vectors and synthetic end-to-end tests pass, but a field capture would provide stronger system-level evidence.
- Bundled TLEs are offline fallbacks dated 2026-09-20 and lose accuracy over time. Users need current imported elements for precise passes.
- Dedicated CW/BFO, WFM stereo/RDS, NOAA APT, HRPT, GOES, and 978 MHz UAT remain explicitly outside current scope.
