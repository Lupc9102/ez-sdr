# ez-sdr Current-Tree Functionality Reconciliation

**Reconciled:** 2026-09-20
**Harness / model:** Codex / GPT-6 / `functions.exec`
**Scope:** Current worktree against `ADSB_DOCS_AUDIT.md`, `LRPT_DOCS_AUDIT.md`, and `FREQUENCY_WEB_SOURCES_AUDIT.md`.

The three detailed audit documents preserve their original defect evidence. Their current-tree resolution tables, this report, and executable tests are authoritative for the present implementation.

## Current status

| Area | Reconciled implementation |
|---|---|
| ADS-B sample ingestion | GUI and daemon preserve overlap across IQ blocks and use cumulative 2.4 MSPS/12 MHz-clock timestamps. |
| ADS-B protocol | Short-message correction order, DF11 syndrome checks, altitude/Gillham, TC19 vertical rate, local/global/surface CPR, antimeridian normalization, and ICAO-filter lifecycle are corrected. Mode A/C replies no longer create phantom aircraft. Scope is explicitly 1090ES; UAT is not claimed. |
| Meteor LRPT channel coding | Receive order is CCSDS Viterbi, optional NRZ-M, CADU sync, native CCSDS RS(255,223) with four-way interleaving, derandomization, and packet reassembly with the VCDU insert zone. |
| Meteor LRPT modulation and images | OQPSK half-symbol reconstruction, eight phase/axis hypotheses, Huffman/quantization/IDCT MCU decoding, and 1568-pixel line assembly are implemented. Workflows default to 80 ksym/s and the decoder remains configurable for 72 ksym/s captures. |
| Satellite tracking | Built-in and user-imported 2LE/3LE files are parsed by `sgp4` and matched to catalog satellites by NORAD ID. TEME state is converted to an Earth-fixed frame for WGS84 observer azimuth/elevation/range, horizon crossings, ground track, and observer-relative Doppler. Built-ins are dated offline fallbacks. |
| LRPT Doppler behavior | Doppler remains visible, while the receiver stays at the nominal downlink center during a pass so repeated hardware retunes do not break Costas-loop lock. |
| General DSP | AM/NFM channel filters precede nonlinear demodulation and decimation. GUI and daemon SSB paths use sideband-selective complex mixing/filtering; WFM is documented as mono. CW references state that an external BFO/decoder is required. |
| Hardware and band plans | HackRF gain is planned across RF amp/LNA/VGA with error rollback. ITU Region 1/2/3 overlays, maritime MF spots, corrected GPS spans, Marine VHF, and split ISS Voice/SSTV/APRS entries replace the audited incorrect claims. |
| Web and external services | Browser LRPT supports Meteor-M2-3 and M2-4. Vertical rate is decoded and displayed. Plaintext IP geolocation and local DuckDuckGo HTML scraping were removed; provider-native search is disclosed. Planespotters parsing/image URLs and OSM attribution are corrected. |

## Validation

- `cargo fmt --all`: passed.
- `cargo test -p lrpt-decode`: 65 passed.
- `cargo test -p dump1090`: 246 library tests and 246 binary-target tests passed.
- `cargo test -p ez-gui --lib`: 476 passed, including localhost tests when run outside the socket-restricted sandbox.
- `cargo test --workspace`: passed, including integration and documentation tests.
- `cargo check --workspace --all-features`: passed. The host lacks optional `libhackrf` and SoapySDR pkg-config packages, so their expected build-script warnings remain.
- `ez-web` `npm test -- --run`: 3 suites passed.
- `ez-web` `npm run build`: passed.

## Remaining evidence limits

- No physical RTL-SDR, HackRF, or SoapySDR device was available. Compile, synthetic, replay, and protocol tests do not establish RF sensitivity or driver behavior on a specific host.
- No independently sourced off-air Meteor IQ fixture with a known decoded image is checked into the repository. The implemented layers have independent vectors and targeted regressions, but an end-to-end field capture remains valuable validation.
- Bundled TLEs age. Accurate pass timing requires current imported elements; automatic TLE download is not implemented or claimed.
- Dedicated CW/BFO, WFM stereo/RDS, NOAA APT, HRPT, GOES, and 978 MHz UAT remain outside the stated feature scope.

## Assessment

The implementation defects identified by the named audits are reconciled in the current tree. Remaining items are field-validation and explicitly documented product-scope limits rather than the protocol and DSP omissions described by the original audit prose.
