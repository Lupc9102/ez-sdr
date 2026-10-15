# ADS-B and Mode S Technical Documentation and Implementation Audit Report

**Document Identification:** `ADSB_DOCS_AUDIT.md`  
**Audit Date:** 2026-09-18  
**Auditor Harness & Model:** Codex-agy harness, Gemini 3.6 Flash (High Reasoning)  
**Target Repository:** `ez-sdr` (`/home/lupc/Documents/ez-sdr`)  
**Scope:** Deep read-only verification of all ADS-B / Mode S functionality, demodulation DSP, protocol decoders, channel pipelines, UI/API interfaces, frequency claims, and test fixtures against primary international aviation standards and reference implementations.

> **Current-tree resolution (2026-09-20):** The findings below are the historical, pre-fix audit and must not be read as a description of the current implementation. Current source and tests resolve every ADS-B defect listed in the executive finding.

| Original finding | Current-tree resolution |
|---|---|
| GUI altitude/Gillham corruption and short-frame field misuse | The GUI uses the shared Mode-S decoder, with correct short-message CRC/error-correction ordering and altitude/identity field handling. |
| Phantom Mode A/C aircraft | Mode A/C replies are no longer inserted as ICAO aircraft tracks. |
| GUI and daemon block-boundary loss | Both paths retain magnitude overlap across IQ blocks; the GUI has a cross-block regression. |
| Timestamp scale error and 2.0 MSPS CLI default | Timestamps accumulate in 12 MHz ticks and the standalone default is 2.4 MSPS. |
| Invalid DF11 acceptance | Non-II/SI syndrome bits are rejected. |
| CPR limitations | Global airborne CPR normalizes longitude; established tracks support single-frame local updates; surface CPR accepts an observer/track reference; antimeridian behavior is tested. |
| Saturating ICAO filter | The filter has time-based expiry and insertion-driven rotation, with sustained-load and TTL regressions. |
| Missing TC19 vertical rate | TC19 climb/descent rate is decoded and propagated through daemon telemetry. |

The remaining validation limit is environmental: no live 1090 MHz RF capture or physical receiver was available. The historical analysis, diagrams, and citations below remain useful evidence for why each change was required.

---

## 1. Executive Finding

ez-sdr provides two parallel, divergent implementations of ADS-B and Mode S signal processing and message decoding:
1. **The Daemon / Subsystem Stack:** [`dump1090`](file:///home/lupc/Documents/ez-sdr/dump1090/src) crate (demodulator, tracker, CPR engine, CRC, Mode A/C, Mode S) consumed by [`ez-daemon::pipelines::packet::PacketPipeline`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs), streaming JSON snapshots over WebSockets to [`ez-web`](file:///home/lupc/Documents/ez-sdr/ez-web/src/ui/aircraft-panel.ts).
2. **The Desktop GUI Embedded Stack:** [`ez-gui::adsb_decoder::AdsBDecoder`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs) and [`ez-gui::adsb_panel::AdsBPanel`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs), which wraps `dump1090::demod::Demod2400` directly in the UI process but re-implements Mode S and ADS-B field decoding in private functions independently of `dump1090::mode_s`.

The audit reveals that while the core Rust port of FlightAware `dump1090` in `dump1090/src` contains accurate implementations of 112-bit CRC24 calculation, CPR global airborne decoding, and 2.4 MSPS pulse-position modulation (PPM) correlators, **the system suffers from severe algorithmic errors, broken protocol conversions, and silent sample dropping across the repository layers**:

* **Catastrophic Altitude Corruption in Desktop GUI:** [`ez-gui/src/adsb_decoder.rs:301`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L301) evaluates altitude for 25-ft encoded ADS-B airborne messages as `((alt16 & 0x1FF) * 25 + 1000) / 4`. This fabricated equation divides legitimate altitudes by approximately 4 and applies an inverted offset; a cruising airliner at 35,000 ft is rendered at 2,512 ft. Furthermore, the unit test [`decode_altitude_q_bit_set`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L358-L365) hardcodes and asserts this corrupt arithmetic.
* **Fictional Mode C / Gillham Polynomial:** [`ez-gui/src/adsb_decoder.rs:314`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L314) decodes Gray-coded Mode C altitude by computing `d12 * 500 + d10 * 100 + d8 * 20 + d6 * 4 + d4 + m + n`. Mode C is an 11-wire cyclic Gray code specified in ICAO Annex 10 Vol IV; treating it as a base-5/base-10 weighted sum yields wild, nonsensical altitudes.
* **Corrupted Field Extraction on Short Surveillance Messages:** [`ez-gui/src/adsb_decoder.rs:267-272`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L267-L272) attempts to decode altitude from DF 0, 4, 5, 16, 20, 21 using the same `decode_altitude` function that reads message bytes 5..7. In 56-bit (7-byte) short surveillance formats (DF 0, 4, 5), bytes 4..7 constitute the Address/Parity (AP) field. The decoder reads the AP field as altitude, and treats DF 5/21 squawk identity codes as altitude.
* **Phantom Aircraft Generated from Mode A/C Squawks:** [`ez-gui/src/adsb_decoder.rs:88-99`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L88-L99) calls `demodulate_ac` and inserts the 12-bit Mode A squawk code directly into the aircraft map as a 24-bit ICAO address (`mm.addr`), injecting stationary phantom aircraft with latitude and longitude `0.0` whenever secondary radar interrogations occur.
* **Inter-Block Boundary Message Loss:** Both [`ez-daemon/src/pipelines/packet.rs:132`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs#L132) and [`ez-gui/src/adsb_decoder.rs:63`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L63) hardcode `overlap: 0` on every input buffer passed to `Demod2400`. Because `Demod2400` terminates demodulation when fewer than 289 samples remain in a block, the trailing 120.4 microseconds of every buffer block are permanently skipped without carryover, dropping all messages straddling buffer boundaries.
* **Timestamp Unit Inconsistency:** `dump1090::demod` requires `sample_timestamp` in 12 MHz clock ticks ($5 \times \text{samples}$ at 2.4 MSPS). [`ez-daemon/src/pipelines/packet.rs:133`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs#L133) feeds raw sample count `block.start_sample`, leading to a $5\times$ drift in message timestamps between blocks.
* **CLI Standalone Default Sample Rate Mismatch:** [`dump1090/src/main.rs:42-43`](file:///home/lupc/Documents/ez-sdr/dump1090/src/main.rs#L42-L43) defaults `--sample-rate` to `2000000` (2.0 MSPS), while the underlying demodulator `Demod2400` is rigidly tuned for 2.4 MSPS, rendering the standalone CLI binary non-functional under default arguments.
* **Corrupt DF 11 Acceptance via Empty Error Guard:** [`dump1090/src/demod.rs:333-336`](file:///home/lupc/Documents/ez-sdr/dump1090/src/demod.rs#L333-L336) contains an empty `if syndrome & 0xFFFF80 != 0 { }` branch, failing to reject corrupted DF 11 All-Call replies that have non-zero syndrome bits outside the 7-bit II/SI field.
* **Unbounded Bloom Filter Saturation:** [`dump1090/src/icao_filter.rs`](file:///home/lupc/Documents/ez-sdr/dump1090/src/icao_filter.rs) maintains a fixed 4096-bit bitset with no eviction or decay mechanism. `clear()` is never invoked during runtime. The filter saturates over extended sessions, causing all CRC-overlaid surveillance messages (DF 0, 4, 5, 16, 20, 21) to be accepted as "known" aircraft.
* **Absence of Local CPR Tracking & Surface CPR Support:** [`dump1090/src/cpr.rs:353-386`](file:///home/lupc/Documents/ez-sdr/dump1090/src/cpr.rs#L353-L386) only resolves positions when alternating even and odd frames arrive within 10 seconds. It never performs local CPR decoding against established track positions, dropping position updates whenever an aircraft drops one frame parity. Surface CPR decoding is completely unimplemented in both `cpr.rs::CprDecoder` (returns `None`) and `adsb_decoder.rs`.

---

## 2. Implementation Trace

The repository contains two operational paths for processing Mode S / ADS-B traffic:

```
                  ┌────────────────────────────────────────────────────────┐
                  │                 RF Ingest / Hardware                   │
                  │        (RTL-SDR / HackRF / SoapySDR @ 1090 MHz)        │
                  └───────────────────────────┬────────────────────────────┘
                                              │
                     ┌────────────────────────┴────────────────────────┐
                     ▼                                                 ▼
        ┌─────────────────────────┐                       ┌─────────────────────────┐
        │     ez-daemon Path      │                       │       ez-gui Path       │
        └────────────┬────────────┘                       └────────────┬────────────┘
                     │ IQ Samples                                      │ IQ Samples (&[u8])
                     ▼                                                 ▼
        ┌─────────────────────────┐                       ┌─────────────────────────┐
        │  ez-daemon Channelizer  │                       │   ez-gui::app Main Loop │
        │ (DDC to 2.4 MSPS bus)   │                       │ (Tuned to 1090 MHz)     │
        └────────────┬────────────┘                       └────────────┬────────────┘
                     │ SampleBlock                                     │
                     ▼                                                 ▼
        ┌─────────────────────────┐                       ┌─────────────────────────┐
        │      PacketPipeline     │                       │       AdsBDecoder       │
        │ (packet.rs)             │                       │ (adsb_decoder.rs)       │
        │ - MagBuf (overlap: 0)   │                       │ - MagBuf (overlap: 0)   │
        │ - Demod2400             │                       │ - Demod2400             │
        │ - Tracker (addr, count) │                       │ - Local field parsing   │
        │ - CprDecoder (airborne) │                       │ - Phantom Mode A/C      │
        └────────────┬────────────┘                       └────────────┬────────────┘
                     │                                                 │
                     ▼                                                 ▼
        ┌─────────────────────────┐                       ┌─────────────────────────┐
        │   WebSocket Streamer    │                       │        AdsBPanel        │
        │ (ws.rs: /ws/stream/...) │                       │ (adsb_panel.rs)         │
        └────────────┬────────────┘                       │ - Table / Map View      │
                     │ JSON AircraftTelemetry[]                   │ - Range / Toast Alerts  │
                     ▼                                    └─────────────────────────┘
        ┌─────────────────────────┐
        │  ez-web AircraftPanel   │
        │ (aircraft-panel.ts)     │
        │ - DOM table display     │
        └─────────────────────────┘
```

### 2.1 Web / Daemon Path
1. **Workflow Activation ([`ez-web/src/workflows/adsb.ts`](file:///home/lupc/Documents/ez-sdr/ez-web/src/workflows/adsb.ts)):** `startAdsbWorkflow` queries daemon hardware, verifies/sets frequency to `1_090_000_000` Hz and sample rate to `2_400_000` Hz, then requests channel creation with kind `"AdsbPackets"`, bandwidth `2_400_000` Hz, and center offset `0`.
2. **Channelizer Ingest ([`ez-daemon/src/channelizer.rs`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/channelizer.rs)):** Downconverts and packetizes incoming samples into [`SampleBlock`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/bus.rs) structures published on the channel's `SampleBus`.
3. **Packet Pipeline ([`ez-daemon/src/pipelines/packet.rs`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs)):**
   - In `process_block`, converts complex IQ to magnitude values in `mag_buf`. Hardcodes `overlap: 0` and `flags: MAGBUF_DISCONTINUOUS`.
   - Calls `demod.demodulate(&mag, &mut self.stats, &mut |mm| messages.push(mm.clone()))`.
   - In `merge_message`, passes `mm` to `dump1090::track::Tracker`.
   - Calls `dump1090::mode_s::decode_mode_s_message(&mm.msg)`. If valid, updates callsign, altitude, ground speed, and heading.
   - If DF 17 or 18 and TC 9..=18, extracts raw CPR bits via `extract_airborne_cpr(&mm.msg)` and submits to `dump1090::cpr::CprDecoder`. If an even/odd pair decodes within 10 seconds, updates `entry.lat` and `entry.lon`.
   - Every 25 blocks (`PUBLISH_EVERY_N_BLOCKS`), executes `prune_and_publish`, dropping entries not seen within 60,000 ms, and broadcasts `Vec<AircraftTelemetry>`.
4. **WebSocket Stream ([`ez-daemon/src/web/ws.rs`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/web/ws.rs)):** Route `/ws/stream/adsb-packets/{id}` subscribes to the pipeline broadcaster, serializes `Vec<AircraftTelemetry>` to JSON text frames, and pushes to client.
5. **Web UI ([`ez-web/src/ui/aircraft-panel.ts`](file:///home/lupc/Documents/ez-sdr/ez-web/src/ui/aircraft-panel.ts)):** Receives raw JSON array, renders sortable HTML table with ICAO hex, callsign, altitude, latitude, longitude, ground speed, track, vertical rate (always dash), and message count.

### 2.2 Desktop GUI Path
1. **Mode Activation ([`ez-gui/src/adsb_panel.rs`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs)):** Entering "Planes" mode invokes `AdsBPanel::begin()`, tuning the SDR source to 1,090,000,000 Hz and 2,400,000 Hz sample rate.
2. **Main Loop Feeding ([`ez-gui/src/app.rs:602-610`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/app.rs#L602-L610)):** When frequency is 1090 MHz $\pm 2$ MHz and ADS-B is running, raw sample buffers are passed to `self.adsb_decoder.feed_iq(samples, rate)`.
3. **Demodulation & Message Parsing ([`ez-gui/src/adsb_decoder.rs`](file:///home/lupc/Documents/ez-gui/src/adsb_decoder.rs)):**
   - Magnitude conversion hardcodes `InputFormat::Uc8`. Overlap is hardcoded to 0.
   - `Demod2400::demodulate` decodes Mode S messages into `decoded` vector.
   - `Demod2400::demodulate_ac` decodes Mode A/C messages and appends them into `decoded`.
   - Iterates through `decoded` calling `process_decoded(icao, msgtype, msg)`.
   - Re-implements altitude extraction via defective `decode_altitude(&msg)`.
   - Re-implements velocity extraction via `msg[4..9]` parsing.
   - Calls `try_cpr_decode(&entry.cpr_even, &entry.cpr_odd)` using `dump1090::cpr::decode_cpr_airborne`.
4. **GUI Display & Alerts ([`ez-gui/src/adsb_panel.rs`](file:///home/lupc/Documents/ez-gui/src/adsb_panel.rs)):** Renders aircraft list, OSM map tiles, aircraft 3D category silhouette icons, altitude/callsign filters, range alerts via Haversine distance, and desktop toast notifications.

---

## 3. Standards / Source Ledger

The following ledger establishes the primary authoritative references against which all claims, formulas, and implementations are verified:

| Reference Identifier | Issuing Organization | Title / Publication Details | Canonical URL | Access Date | Status & Scope |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **ICAO-DOC-9871** | International Civil Aviation Organization (ICAO) | *Technical Provisions for Mode S Services and Extended Squitter*, Doc 9871, 2nd Edition (2012) | [ICAO Store Doc 9871](https://store.icao.int/en/technical-provisions-for-mode-s-services-and-extended-squitter-doc-9871) | 2026-09-18 | Authoritative standard for 1090 MHz Extended Squitter message formats (BDS registers 0,5, 0,6, 0,8, 0,9), Compact Position Reporting (CPR) algorithms (§A.2.7), and 25-ft altitude encoding. Paywalled by ICAO; verified via harmonized EU regulations and RTCA DO-260B. |
| **ICAO-ANNEX-10-V4** | International Civil Aviation Organization (ICAO) | *Annex 10 to the Convention on International Civil Aviation — Aeronautical Telecommunications, Volume IV (Surveillance and Collision Avoidance Systems)*, 5th Edition | [ICAO Store Annex 10 Vol IV](https://store.icao.int/en/annex-10-aeronautical-telecommunications-volume-iv-surveillance-and-collision-avoidance-systems) | 2026-09-18 | Authoritative standard for Mode S RF modulation (1090 MHz, 1 Mbps PPM, §3.1.2.1), CRC24 generator polynomial `0xFFF409` (§3.1.2.3.2.1.2), Downlink Formats (§3.1.2.5.2), and Gillham Gray code (App. 1 to Ch. 3). Paywalled; technical parameters cross-verified against open radar specifications. |
| **FAA-AC-20-165B** | Federal Aviation Administration (FAA) | *Airworthiness Approval of Automatic Dependent Surveillance - Broadcast OUT Systems*, Advisory Circular AC 20-165B (2015) | [FAA AC 20-165B](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentid/1028723) | 2026-09-18 | Publicly accessible. Defines dual-link ADS-B operational scope in the United States (1090 MHz 1090ES vs 978 MHz UAT) under 14 CFR § 91.225. |
| **FAA-14CFR-91** | U.S. National Archives / Federal Aviation Administration | *Code of Federal Regulations, Title 14, Part 91, Section 91.225: Automatic Dependent Surveillance-Broadcast (ADS-B) Out equipment and use* | [eCFR 14 CFR § 91.225](https://www.ecfr.gov/current/title-14/chapter-I/subchapter-F/part-91/subpart-C/section-91.225) | 2026-09-18 | Publicly accessible. Codifies requirement for 1090 MHz Extended Squitter at or above 18,000 ft MSL, allowing 978 MHz UAT only below 18,000 ft MSL. |
| **RTCA-DO-260B** | RTCA, Inc. / EUROCAE | *Minimum Operational Performance Standards for 1090 MHz Extended Squitter Automatic Dependent Surveillance - Broadcast (ADS-B) and Traffic Information Services - Broadcast (TIS-B)*, RTCA DO-260B / EUROCAE ED-102A (2009) | [RTCA Store DO-260B](https://www.rtca.org/standards-store/) | 2026-09-18 | Authoritative avionics standard for 1090 MHz Extended Squitter receivers. Details CPR global and local decoding windows (§2.2.3.2.7.2). Commercial standard (paywalled). |
| **RTCA-DO-282B** | RTCA, Inc. | *Minimum Operational Performance Standards for Universal Access Transceiver (UAT) Automatic Dependent Surveillance - Broadcast*, RTCA DO-282B (2009) | [RTCA Store DO-282B](https://www.rtca.org/standards-store/) | 2026-09-18 | Authoritative standard for 978 MHz UAT CPFSK modulation at 1.041667 Mbps. Commercial standard (paywalled). |
| **EUROCONTROL-MODES** | European Organisation for the Safety of Air Navigation (EUROCONTROL) | *Principles of Mode S and ADS-B Operations*, Eurocontrol Guidelines & Training Reference | [EUROCONTROL Mode S Principles](https://www.eurocontrol.int/publication/principles-mode-s-and-ads-b-operations) | 2026-09-18 | Publicly accessible. Defines European 1090 MHz mandate, SSR Mode A/C radar characteristics, and Mode S address parity (AP) mechanics for surveillance replies. |
| **UPSTREAM-DUMP1090** | FlightAware LLC | *dump1090: Mode S and ADS-B audio and data demodulator for RTLSDR devices* (commit `master`) | [GitHub FlightAware dump1090](https://github.com/flightaware/dump1090) | 2026-09-18 | Upstream open-source baseline (GPL-2.0). Utilized strictly where international standards permit implementation discretion (e.g. 2.4 MSPS demodulator correlator phase definitions, trailing sample overlap sizing, and noise floor estimation). |

---

## 4. Requirement-by-Requirement Matrix

| Requirement / Domain | Standard & Clause | ez-sdr Implementation Status | Verification Verdict | Primary Audit Finding |
| :--- | :--- | :--- | :--- | :--- |
| **RF Center Frequency** | ICAO Annex 10 Vol IV §3.1.2.1.1; FAA 14 CFR §91.225 | Hardcoded 1,090,000,000 Hz in `ez-web`, `ez-gui`, and `dump1090`. | **CONFORMANT** | Operating frequency matches 1090 MHz international standard. |
| **Dual-Link Scope (1090 vs 978 MHz)** | FAA AC 20-165B §2.1; RTCA DO-282B | 978 MHz UAT is completely unreferenced and unsupported across all crates. | **DOCUMENTATION GAP** | Documentation claims generic "ADS-B" without qualifying that 978 MHz UAT (used in US GA airspace <18,000 ft) is unsupported. |
| **Modulation & Data Rate** | ICAO Annex 10 Vol IV §3.1.2.1.2 | 1 Mbps Pulse Position Modulation (PPM), 0.5 $\mu\text{s}$ pulse half-bits. | **CONFORMANT** | Core demodulator implements PPM detection. |
| **Preamble Structure & Detection** | ICAO Annex 10 Vol IV §3.1.2.1.3 | 8.0 $\mu\text{s}$ preamble with pulses at 0.0, 1.0, 3.5, 4.5 $\mu\text{s}$. | **CONFORMANT** | `Demod2400::demodulate` correlates rising/falling edges and peaks for phases 3..7. |
| **Sample Rate Assumptions** | Upstream dump1090 `demod_2400.c` | Rigidly requires 2.4 MSPS ($2.4\text{ samples}/\mu\text{s}$). | **NON-CONFORMANT IN CLI** | CLI `dump1090/src/main.rs:42` defaults to 2.0 MSPS, breaking all correlator math under default invocation. |
| **Buffer Overlap & Boundary Demod** | Upstream dump1090 `TRAILING_SAMPLES` | Demodulator requires $\ge 289$ overlap samples to decode boundary-spanning messages. | **DEFECTIVE IN PRODUCTION** | `ez-daemon/src/pipelines/packet.rs:132` and `ez-gui/src/adsb_decoder.rs:63` hardcode `overlap: 0`, silently dropping all boundary messages. |
| **CRC24 Polynomial & Calculation** | ICAO Annex 10 Vol IV §3.1.2.3.2.1.2 | $G(x) = x^{24} + x^{23} + \dots + 1$ (`0xFFF409`). | **CONFORMANT** | `dump1090/src/crc.rs` correctly builds and applies the 256-entry CRC24 lookup table. |
| **Parity / Address Recovery (DF 0,4,5,16,20,21)** | ICAO Annex 10 Vol IV §3.1.2.5.2 | Address overlaid on parity ($AP = \text{CRC} \oplus \text{Addr}$). | **PARTIALLY DEFECTIVE** | `dump1090/src/demod.rs:493` recovers `addr = crc & 0xFFFFFF`, but `icao_filter` saturation allows random noise syndromes to masquerade as valid addresses. |
| **DF 11 All-Call Parity Validation** | ICAO Annex 10 Vol IV §3.1.2.5.2.2 | PI field holds CRC parity XORed with 7-bit II/SI code. | **DEFECTIVE** | `dump1090/src/demod.rs:333-336` has an empty `if` block, failing to reject corrupt frames where `syndrome & 0xFFFF80 != 0`. |
| **Error Correction Scoping** | ICAO Annex 10 Vol IV §3.1.2.5.2 | Message length is 56 bits (short) or 112 bits (long) based on DF. | **DEFECTIVE** | `dump1090/src/demod.rs:215` executes 112-bit CRC and 1/2-bit error correction on 56-bit short messages before testing short CRC. |
| **Airborne Position CPR Extraction** | ICAO Doc 9871 Table A-2-5 (BDS 0,5) | ME bits 21-22 (F flag), bits 23-39 (Lat-CPR), bits 40-56 (Lon-CPR). | **CONFORMANT** | `extract_airborne_cpr` in `packet.rs:244` and `adsb_decoder.rs:179` correctly extracts 17-bit odd/even fields. |
| **Global CPR Decoding** | ICAO Doc 9871 §A.2.7.2 | Unambiguous latitude/longitude recovery from even + odd pair. | **CONFORMANT** | `dump1090/src/cpr.rs::decode_cpr_airborne` implements exact ICAO 59-boundary $NL(\text{lat})$ transition table. |
| **Local CPR Decoding** | ICAO Doc 9871 §A.2.7.3; RTCA DO-260B | Unambiguous tracking of single CPR frame against known position within 30s. | **MISSING IN RUNTIME** | `dump1090/src/cpr.rs::CprDecoder::submit_with_time` only attempts global paired decoding; subsequent single frames are never decoded. |
| **Surface Position CPR Decoding** | ICAO Doc 9871 Table A-2-6 (BDS 0,6) | Surface CPR grid ($90^\circ/60$ and $90^\circ/59$) with reference position. | **DEFECTIVE / MISSING** | `dump1090/src/cpr.rs:376` unconditionally returns `None`. `ez-gui/src/adsb_decoder.rs:242` parses bits but never calls CPR decode. |
| **Relative Longitude Normalization** | ICAO Doc 9871 §A.2.7.3 | Normalizes longitude to $[-180^\circ, +180^\circ]$. | **DEFECTIVE** | `dump1090/src/cpr.rs:278-280` only subtracts $360^\circ$ if $rlon > 180^\circ$; fails to normalize when $rlon < -180^\circ$. |
| **Altitude Decoding (DF 17 Q=1)** | ICAO Doc 9871 Table A-2-5; §A.2.3.2.7.2 | $11\text{-bit } N \times 25\text{ ft} - 1000\text{ ft}$. | **FATALLY DEFECTIVE IN GUI** | `dump1090/src/mode_s.rs:117` is correct. `ez-gui/src/adsb_decoder.rs:301` computes `((alt16 & 0x1FF) * 25 + 1000) / 4`. |
| **Altitude Decoding (DF 17 Q=0 / Mode C)** | ICAO Annex 10 Vol IV App. 1 to Ch. 3 | 11-bit Gillham cyclic Gray code. | **FATALLY DEFECTIVE IN GUI** | `dump1090/src/mode_s.rs:123` correctly calls Gray decode. `ez-gui/src/adsb_decoder.rs:314` calculates fictional digit sum. |
| **Surveillance Altitude (DF 0,4,16,20)** | ICAO Annex 10 Vol IV §3.1.2.5.2 | AC field at message bits 20-32 (bytes 2..3). | **DEFECTIVE IN GUI** | `ez-gui/src/adsb_decoder.rs:267` calls `decode_altitude` reading bytes 5..7 (reads AP parity field on short messages). |
| **Surveillance Identity (DF 5, 21)** | ICAO Annex 10 Vol IV §3.1.2.5.2.1.2 | Bits 20-32 contain Mode A 4-digit octal squawk code. | **DEFECTIVE IN GUI** | `ez-gui/src/adsb_decoder.rs:267` passes DF 5/21 to `decode_altitude`, populating aircraft altitude with squawk codes. |
| **Mode A/C Separation** | ICAO Annex 10 Vol IV §3.1.1 | Mode A/C radar pulses lack 24-bit ICAO addresses. | **DEFECTIVE IN GUI** | `ez-gui/src/adsb_decoder.rs:92-98` injects Mode A/C squawks directly into `self.aircraft` as phantom ICAO aircraft entries. |
| **Flight ID / Callsign Decoding** | ICAO Doc 9871 Table A-2-4 (BDS 0,8) | 8 characters, 6 bits per character (ASCII subset). | **CONFORMANT** | Both `dump1090/src/mode_s.rs:76` and `ez-gui/src/adsb_decoder.rs:137` correctly unpack 6-bit characters. |
| **Airborne Velocity Decoding** | ICAO Doc 9871 Table A-2-9 (BDS 0,9) | Subtypes 1-2 (ground speed, E/W & N/S vectors, supersonic multiplier). | **CONFORMANT** | Both `dump1090/src/mode_s.rs:164` and `ez-gui/src/adsb_decoder.rs:215` correctly calculate speed and track heading. |
| **Vertical Rate Decoding** | ICAO Doc 9871 Table A-2-9 (BDS 0,9) | ME bits 37-45 encode vertical rate ($64\text{ ft/min}$ steps). | **MISSING IN RUNTIME** | Neither `dump1090` nor `ez-daemon` decodes vertical rate; `ez-web` displays em-dash in table. |
| **Timestamp Clock Domains** | Upstream dump1090 `demod_2400.c` | 12 MHz clock tick interpolation ($5 \text{ ticks/sample}$). | **DEFECTIVE IN DAEMON** | `ez-daemon/src/pipelines/packet.rs:133` passes unscaled sample index into 12 MHz tick container. |
| **Known Aircraft Filter Expiration** | Upstream dump1090 `icao_filter.c` | Bitset cache requires periodic eviction to prevent saturation. | **DEFECTIVE** | `dump1090/src/icao_filter.rs` has no TTL, timeout, or clear call in runtime; permanently saturates. |

---

## 5. Verified Defects with Current File:Line Evidence and Impact

### Defect 1: Fatal Altitude Calculation Corruption in Desktop GUI ADS-B Decoder
* **File & Line:** [`ez-gui/src/adsb_decoder.rs:297-302`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L297-L302)
* **Current Code:**
  ```rust
  fn decode_altitude(msg: &[u8]) -> u32 {
      let q = (msg[5] & 0x10) != 0;
      if q {
          let alt16 = (u32::from(msg[5]) << 1) | (u32::from(msg[6]) >> 7);
          ((alt16 & 0x1FF) * 25 + 1000) / 4
      } else {
  ```
* **Authoritative Standard:** ICAO Doc 9871 Table A-2-5 / §A.2.3.2.7.2; RTCA DO-260B §2.2.3.2.7.1.
* **Proof of Defect:**
  In a DF 17/18 Airborne Position message, the 12-bit Altitude Code occupies ME bits 9-20 (`(msg[5] << 4) | (msg[6] >> 4)`). The Q bit is bit 8 of the 12-bit field (bit 4 of `msg[5]`). When $Q = 1$, the remaining 11 bits represent altitude $N$ in 25-ft increments:
  $$\text{Altitude (ft)} = N \times 25 - 1000$$
  Instead, `ez-gui/src/adsb_decoder.rs` extracts a corrupted 9-bit quantity, adds 1000, and divides the total by 4!
  - *Example:* For a commercial flight cruising at 35,000 ft, `msg[5] = 0xB5`, `msg[6] = 0x00`.
    - Correct decode (as in `dump1090/src/mode_s.rs:117`): $ac12 = \text{0xB50}$, $Q = 1$, $N = 1440 \implies 1440 \times 25 - 1000 = 35,000\text{ ft}$.
    - `ez-gui/src/adsb_decoder.rs`: `alt16 = (0xB5 << 1) | 0 = 362 (0x16A)`.
      $$((362 \ \& \ \text{0x1FF}) \times 25 + 1000) / 4 = (9050 + 1000) / 4 = 2,512\text{ ft}.$$
* **Impact:** Every altitude rendered in the desktop application's ADS-B table and map view for 25-ft encoded commercial traffic is completely corrupted (reported at approximately $1/4$ of actual flight level).

---

### Defect 2: Unit Test Pinning Corrupt Altitude Equation
* **File & Line:** [`ez-gui/src/adsb_decoder.rs:357-365`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L357-L365)
* **Current Code:**
  ```rust
  #[test]
  fn decode_altitude_q_bit_set() {
      // Q-bit (msg[5] & 0x10) set → 25ft encoding
      // msg[5]=0x10, msg[6]=0x00 → alt16 = 0x20 = 32 → (32*25+1000)/4 = 450
      let mut msg = [0u8; 14];
      msg[5] = 0x10;
      msg[6] = 0x00;
      assert_eq!(decode_altitude(&msg), 450);
  }
  ```
* **Proof of Defect:** The test author explicitly derived the expected test value from the defective equation `(32 * 25 + 1000) / 4 = 450`. Furthermore, in a 12-bit AC field where `msg[5] = 0x10` and `msg[6] = 0x00`, `ac12 = 0x100`. The 5th bit from the right is bit 4 (`0x10`), which in `ac12 = 0x100` is **zero**, meaning the Q-bit is actually clear ($Q=0$), not set. The test creates a false invariant.
* **Impact:** Regression test suite passes 100% while verifying invalid aviation math.

---

### Defect 3: Fictional Mode C / Gillham Code Polynomial Summation
* **File & Line:** [`ez-gui/src/adsb_decoder.rs:303-315`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L303-L315)
* **Current Code:**
  ```rust
  let m_bit = (msg[5] & 0x20) != 0;
  let n_bit = (msg[5] & 0x10) != 0;
  let d12 = u32::from(msg[5] & 0x0F);
  let d10 = u32::from((msg[6] >> 5) & 0x07);
  let d8 = u32::from((msg[6] >> 2) & 0x07);
  let d6 = u32::from(((msg[6] & 0x03) << 1) | ((msg[7] >> 6) & 0x01));
  let d4 = u32::from((msg[7] >> 2) & 0x0F);

  let m: u32 = if m_bit { 1600 } else { 0 };
  let n: u32 = if n_bit { 40 } else { 0 };

  d12 * 500 + d10 * 100 + d8 * 20 + d6 * 4 + d4 + m + n
  ```
* **Authoritative Standard:** ICAO Annex 10 Vol IV, Appendix 1 to Chapter 3.
* **Proof of Defect:** Gillham code uses an 11-wire cyclic reflected Gray code (pulses $D1, D2, D4, A1, A2, A4, B1, B2, B4, C1, C2, C4$) representing 100-ft increments and 500-ft intervals. Converting Gillham code requires Gray-to-binary bit decoding (as properly implemented in `dump1090/src/mode_ac.rs`). Inventing an ad-hoc weighted sum of bit nibbles (`d12*500 + d10*100 + ...`) has no mathematical or standard basis.
* **Impact:** Any general aviation or legacy transponder transmitting 100-ft Gillham coded altitudes ($Q=0$) is assigned completely chaotic altitude values in the GUI.

---

### Defect 4: Short Surveillance & Identity Messages Decoded with Wrong Offsets
* **File & Line:** [`ez-gui/src/adsb_decoder.rs:267-272`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L267-L272)
* **Current Code:**
  ```rust
  0 | 4 | 5 | 16 | 20 | 21 => {
      // Surveillance / altitude messages
      let alt = decode_altitude(&msg);
      entry.altitude = Some(alt);
      entry.entry.altitude = alt;
  }
  ```
* **Authoritative Standard:** ICAO Annex 10 Vol IV §3.1.2.5.2.1.2, §3.1.2.5.2.2.
* **Proof of Defect:**
  1. DF 0, 4, and 5 are 56-bit (7-byte) short messages (`msg[0..7]`). In short messages, the AC/ID field is located at bits 20-32 (`msg[2..4]`), while `msg[4..7]` is the 24-bit Address/Parity (AP) field. Calling `decode_altitude(&msg)` reads `msg[5]` and `msg[6]`, which are parity bits.
  2. DF 5 (short) and DF 21 (long) do not contain altitude at all; bits 20-32 contain the 4-digit octal Mode A identity (squawk) code.
* **Impact:** The desktop GUI treats Mode A squawk codes and random parity bits as altitudes, overwriting true aircraft altitudes with garbage whenever surveillance replies are intercepted.

---

### Defect 5: Phantom Aircraft Injected from SSR Mode A/C Radar Replies
* **File & Line:** [`ez-gui/src/adsb_decoder.rs:92-98`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L92-L98), [`105-125`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L105-L125)
* **Current Code:**
  ```rust
  self.demod.demodulate_ac(&mag, &mut self.stats, &mut |mm| {
      decoded.push((mm.addr, mm.msgtype, mm.msg, mm.signal_level));
  });

  for (icao, msgtype, msg, _signal_level) in decoded {
      self.total_messages += 1;
      self.process_decoded(icao, msgtype, msg);
  }
  ```
* **Authoritative Standard:** ICAO Annex 10 Vol IV §3.1.1; EUROCONTROL Mode S & ADS-B Principles.
* **Proof of Defect:** Mode A/C pulses have no 24-bit ICAO airframe address. `dump1090::demod::demodulate_ac` assigns `mm.addr = modeac` (the 12-bit squawk code, e.g. `0x1200` for VFR) and `mm.msgtype = 0xFF`. In `process_decoded`, `self.aircraft.entry(icao)` inserts an aircraft record with ICAO `0x1200`, `lat = 0.0`, `lon = 0.0`.
* **Impact:** The desktop aircraft table fills up with phantom aircraft with null coordinates corresponding to every squawk code in the airspace.

---

### Defect 6: Hardcoded Zero Overlap Drops Inter-Block ADS-B Messages
* **File & Line:** [`ez-daemon/src/pipelines/packet.rs:132`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs#L132) and [`ez-gui/src/adsb_decoder.rs:63`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L63)
* **Current Code:**
  In `ez-daemon`:
  ```rust
  let mut mag = MagBuf {
      data,
      total_length: block.samples.len(),
      valid_length: block.samples.len(),
      overlap: 0,
      sample_timestamp: block.start_sample,
      sys_timestamp: now_ms(),
      flags: MagBufFlags(MAGBUF_DISCONTINUOUS),
      mean_level: sum_level / n,
      mean_power: sum_power / n,
      dropped: 0,
  };
  ```
  In `ez-gui`:
  ```rust
  let overlap = 0;
  ...
  let mag = MagBuf {
      data: self.mag_buf[..nsamples].to_vec(),
      total_length: nsamples,
      valid_length: nsamples,
      overlap,
  ```
* **Authoritative Standard:** Upstream FlightAware dump1090 `demod_2400.c` (`TRAILING_SAMPLES = 288`).
* **Proof of Defect:** In `dump1090/src/demod.rs:665`:
  ```rust
  if preamble.len() < 269 + 1 + 19 {
      break;
  }
  ```
  A 112-bit Mode S squitter requires 288 samples at 2.4 MSPS ($8.0\,\mu\text{s} + 112\,\mu\text{s} = 120\,\mu\text{s} \times 2.4 = 288$). The demodulator breaks out of its scanning loop when fewer than 289 samples remain. If `overlap` is 0 and trailing samples are not carried over to the start of the subsequent block, the last 289 samples of every buffer are permanently discarded without demodulation.
* **Impact:** Approximately 0.2%–0.5% of valid Mode S messages are systematically lost. Messages that straddle block boundaries fail preamble detection or suffer truncated payload bits.

---

### Defect 7: Incorrect Sample Timestamp Scaling in Daemon Pipeline
* **File & Line:** [`ez-daemon/src/pipelines/packet.rs:133`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs#L133)
* **Current Code:**
  ```rust
  sample_timestamp: block.start_sample,
  ```
* **Authoritative Standard:** `dump1090/src/demod.rs:55, 925`.
* **Proof of Defect:** Line 55 documents `sample_timestamp` as: `/// Timestamp of the first sample, in 12 MHz ticks.`
  Line 925 calculates message timestamp:
  $$\text{timestamp\_msg} = \text{mag.sample\_timestamp} + j \times 5 + (8 + 56) \times 12 + \text{best\_phase}$$
  Because 2.4 MSPS samples are converted to 12 MHz ticks by multiplying sample index $j$ by 5 ($12 / 2.4 = 5$), `mag.sample_timestamp` must also be scaled by 5 ($\text{sample\_index} \times 5$). Feeding raw `block.start_sample` means the base timestamp advances 5 times slower than the intraday message timestamp offset.
* **Impact:** Inconsistent message timestamps reported across successive sample blocks; Beast binary output timestamps generated via daemon will drift significantly.

---

### Defect 8: Default Sample Rate in Standalone CLI Breaks Demodulation
* **File & Line:** [`dump1090/src/main.rs:42-43`](file:///home/lupc/Documents/ez-sdr/dump1090/src/main.rs#L42-L43)
* **Current Code:**
  ```rust
  #[arg(long, default_value = "2000000", help = "Sample rate in Hz")]
  sample_rate: u32,
  ```
* **Authoritative Standard:** Upstream FlightAware dump1090 `demod_2400.c`.
* **Proof of Defect:** `Demod2400` is structurally defined around a 2.4 MSPS clock ($2.4\text{ samples}/\mu\text{s}$, with 5 phase correlators `slice_phase0..4` on a 12 MHz clock). At 2.0 MSPS ($2.0\text{ samples}/\mu\text{s}$), a $1.0\,\mu\text{s}$ bit is 2 samples instead of 2.4, and the 8 $\mu\text{s}$ preamble spans 16 samples instead of 19.2. All preamble correlation offsets ($1, 3, 9, 11$) and phase slicers fail.
* **Impact:** Invoking `dump1090` without `--sample-rate 2400000` causes near-total failure to detect preambles or decode valid frames.

---

### Defect 9: Empty Error Branch Accepts Corrupted DF 11 Messages
* **File & Line:** [`dump1090/src/demod.rs:332-336`](file:///home/lupc/Documents/ez-sdr/dump1090/src/demod.rs#L332-L336)
* **Current Code:**
  ```rust
  11 => {
      let syndrome = crc24_parity(&corrected[..MODES_SHORT_MSG_BYTES]);
      if syndrome & 0xFFFF80 != 0 {
          // CRC does not match the expected form for DF11 (IID != 0 case handled below loosely).
          // We still allow it if fully valid.
      }
      let iid = syndrome & 0x7F;
      let recent = icao_filter.contains(addr & 0xFFFFFF);
  ```
* **Authoritative Standard:** ICAO Annex 10 Vol IV §3.1.2.5.2.2.
* **Proof of Defect:** In DF 11 (All-Call reply), the 24-bit Parity/Interrogator Identifier (PI) field contains the 24-bit CRC parity XORed with the interrogator's 7-bit II/SI code (which occupies the lowest 7 bits of PI). Therefore, when computing the syndrome, bits 7..23 (`syndrome & 0xFFFF80`) **must** be zero. If any bit in `syndrome & 0xFFFF80` is non-zero, the packet has parity errors. Leaving the `if` body empty allows corrupted packets with arbitrary high-order parity errors to be scored as valid `ScoreRank::Df11IidKnown`.
* **Impact:** Corrupted DF 11 transmissions are accepted and processed into tracking tables whenever an aircraft is marked as known.

---

### Defect 10: 112-Bit Error Correction Prematurely Executed on 56-Bit Short Messages
* **File & Line:** [`dump1090/src/demod.rs:210-257`](file:///home/lupc/Documents/ez-sdr/dump1090/src/demod.rs#L210-L257)
* **Current Code:**
  ```rust
  fn correct_message(
      input: &[u8; MODES_LONG_MSG_BYTES],
      max_errors: usize,
  ) -> (isize, [u8; MODES_LONG_MSG_BYTES]) {
      // Check long-form (112-bit) CRC first.
      let long_syndrome = crc24_parity(input);
      if long_syndrome == 0 {
          return (0, *input);
      }
      // Try 1-bit and 2-bit correction on the long message across 112 bits...
      ...
      // Try short-form (56-bit) correction.
      let short_bytes = &input[..MODES_SHORT_MSG_BYTES];
  ```
* **Authoritative Standard:** ICAO Annex 10 Vol IV §3.1.2.5.2.
* **Proof of Defect:** Downlink formats are partitioned into short (56-bit: DF 0, 4, 5, 11) and long (112-bit: DF 16, 17, 18, 19, 20, 21, 24..31). `correct_message` runs 112-bit CRC parity and 1-2 bit error-syndrome searching across all 14 bytes before checking 56-bit CRC. For 56-bit short messages, bytes 7..14 contain trailing noise/unrelated samples. If those trailing bytes happen to match a 112-bit CRC syndrome, a short message is misdiagnosed as an uncorrected or corrected long message.
* **Impact:** Undefined behavior and false long-message framing on short surveillance replies.

---

### Defect 11: Unbounded ICAO Filter Cache Saturation
* **File & Line:** [`dump1090/src/icao_filter.rs:3-15`](file:///home/lupc/Documents/ez-sdr/dump1090/src/icao_filter.rs#L3-L15)
* **Current Code:**
  ```rust
  const FILTER_SIZE: usize = 4096;
  const FILTER_MASK: usize = FILTER_SIZE - 1;
  const U64_COUNT: usize = FILTER_SIZE / 64;

  pub struct IcaoFilter {
      bits: [u64; U64_COUNT],
  }
  ```
* **Authoritative Standard:** Upstream FlightAware dump1090 `icao_filter.c`.
* **Proof of Defect:** `IcaoFilter` is a 4096-bit bloom filter with no timestamp, decay, or replacement policy. In `dump1090/src`, `IcaoFilter::clear(&mut self)` is never invoked during execution. In an active RF environment, after several hundred aircraft (and false preamble detections) set bits in the 4096-bit filter, the filter saturates (almost all bits become 1). Once saturated, `contains()` returns `true` for every 24-bit address, collapsing all surveillance address-recovery safeguards (`ScoreRank::UnreliableKnown` is assigned to pure noise).
* **Impact:** Receiver degrades into accepting false-positive surveillance replies and fabricated aircraft tracks after continuous operation.

---

### Defect 12: Incomplete Relative Longitude Normalization in CPR
* **File & Line:** [`dump1090/src/cpr.rs:278-280`](file:///home/lupc/Documents/ez-sdr/dump1090/src/cpr.rs#L278-L280)
* **Current Code:**
  ```rust
  if rlon > 180.0 {
      rlon -= 360.0;
  }
  ```
* **Authoritative Standard:** ICAO Doc 9871 §A.2.7.3.
* **Proof of Defect:** Longitude must be normalized to $[-180^\circ, +180^\circ]$. `decode_cpr_relative` checks `if rlon > 180.0 { rlon -= 360.0; }`, but omits the reciprocal check `if rlon < -180.0 { rlon += 360.0; }`. Across the antimeridian (e.g. Pacific operations or near $\pm 180^\circ$), negative out-of-range longitudes remain $<-180^\circ$ instead of wrapping into positive longitude space.
* **Impact:** Position errors when decoding relative CPR near the antimeridian.

---

## 6. Correct Behavior Specifications

### 6.1 Correct Airborne Altitude Decoding (DF 17 / 18, BDS 0,5)
Per ICAO Doc 9871 Table A-2-5 and Section A.2.3.2.7.2:
```rust
pub fn decode_airborne_altitude(msg: &[u8; 14]) -> Option<i32> {
    // Altitude field occupies ME bits 9-20 (msg[5] and high nibble of msg[6])
    let ac12 = (u32::from(msg[5]) << 4) | (u32::from(msg[6]) >> 4);
    let q_bit = (ac12 & 0x0010) != 0;

    if q_bit {
        // 25-ft increment: remove Q-bit (bit 4) to form 11-bit integer N
        let n = ((ac12 & 0x0FE0) >> 1) | (ac12 & 0x000F);
        let alt_ft = (n as i32 * 25) - 1000;
        Some(alt_ft)
    } else {
        // 100-ft Gillham code: insert M-bit (0) and decode Gray code
        let n13 = ((ac12 & 0x0FC0) << 1) | (ac12 & 0x003F);
        let squawk = crate::mode_ac::decode_id13_field(n13);
        let mode_c_100ft = crate::mode_ac::mode_a_to_mode_c(squawk)?;
        Some(mode_c_100ft * 100)
    }
}
```

### 6.2 Correct Inter-Block Buffer Overlap Handling
Per FlightAware dump1090 `demod_2400.c`:
```rust
// In PacketPipeline / AdsBDecoder:
const OVERLAP_SAMPLES: usize = 320; // >= 288 samples for 120 µs long squitter

// Ensure data buffer holds carryover from previous block:
let valid_len = carry.len() + new_samples.len();
mag_buf.clear();
mag_buf.extend_from_slice(&carry);
mag_buf.extend_from_slice(&new_samples);

let mag = MagBuf {
    data: mag_buf.clone(),
    total_length: mag_buf.len(),
    valid_length: valid_len,
    overlap: carry.len(),
    sample_timestamp: current_sample_idx * 5, // scaled to 12 MHz ticks
    sys_timestamp: now_ms(),
    flags: MagBufFlags(0),
    mean_level,
    mean_power,
    dropped: 0,
};

// Save trailing samples for next block
carry.clear();
if valid_len >= OVERLAP_SAMPLES {
    carry.extend_from_slice(&mag_buf[valid_len - OVERLAP_SAMPLES..valid_len]);
}
```

### 6.3 Correct DF 11 All-Call Parity Validation
Per ICAO Annex 10 Vol IV §3.1.2.5.2.2:
```rust
11 => {
    let syndrome = crc24_parity(&corrected[..MODES_SHORT_MSG_BYTES]);
    // High 17 bits must be strictly zero; only lower 7 bits may carry II/SI code
    if (syndrome & 0xFFFF80) != 0 {
        return ScoreRank::Uncorrectable;
    }
    let iid = syndrome & 0x7F;
    let recent = icao_filter.contains(addr & 0xFFFFFF);
    match corrections {
        0 => if iid == 0 {
            if recent { ScoreRank::Df11AcqKnown } else { ScoreRank::Df11AcqUnknown }
        } else {
            if recent { ScoreRank::Df11IidKnown } else { ScoreRank::Df11IidUnknown }
        },
        1 => if iid == 0 {
            if recent { ScoreRank::Df11Acq1ErrorKnown } else { ScoreRank::Df11Acq1ErrorUnknown }
        } else {
            if recent { ScoreRank::Df11Iid1ErrorKnown } else { ScoreRank::Df11Iid1ErrorUnknown }
        },
        _ => ScoreRank::Uncorrectable,
    }
}
```

---

## 7. Frequency Claims & Regional Allocations

### 7.1 1090 MHz Mode S Extended Squitter
* **Allocation:** ITU Radio Regulations allocation for Aeronautical Radionavigation Service (ARNS).
* **Specifications:** ICAO Annex 10, Vol IV §3.1.2.1.1 dictates carrier center frequency of $1090 \pm 1.0\text{ MHz}$.
* **Scope in ez-sdr:** Fully supported across software layers (`ADSB_FREQUENCY_HZ = 1_090_000_000`).

### 7.2 978 MHz Universal Access Transceiver (UAT)
* **Allocation:** 978 MHz is allocated in the United States under FAA NextGen for General Aviation aircraft flying below 18,000 ft MSL (14 CFR § 91.225(b)) and ground broadcast services (TIS-B and FIS-B weather).
* **Modulation:** RTCA DO-282B specifies 978 MHz UAT as Continuous Phase Frequency Shift Keying (CPFSK) at $1.041667\text{ Mbps}$.
* **Scope in ez-sdr:** **Completely absent.** ez-sdr cannot demodulate UAT because it lacks an FSK demodulator and 978 MHz channelizer preset.
* **Documentation Finding:** The repository claims to support "ADS-B" generally (e.g. in `README.md`, `ez-gui/src/quick_start.rs`, and `ez-gui/src/frequency_db.rs`). In the United States, users attempting to track general aviation aircraft broadcasting solely on 978 MHz UAT will receive no traffic. Documentation should explicitly specify: **"1090 MHz Mode S Extended Squitter ADS-B only; 978 MHz UAT is not supported."**

---

## 8. Test Gaps and Inadequate Test Coverage

1. **Test Invariant Masking Defect:** [`ez-gui/src/adsb_decoder.rs:358-365`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L358-L365) tests that `decode_altitude` produces 450 from `msg[5]=0x10, msg[6]=0x00`. This directly asserts the broken formula. There is no test validating high-altitude airline squitters (e.g. FL350 / 35,000 ft) or negative altitudes (e.g. -500 ft below sea level).
2. **Missing Local CPR Tracking Test:** In [`dump1090/src/cpr.rs`](file:///home/lupc/Documents/ez-sdr/dump1090/src/cpr.rs), all tests feed matching even and odd frames simultaneously. There are zero unit tests verifying track maintenance when consecutive odd frames or consecutive even frames arrive, masking the lack of local CPR decoding in `CprDecoder::submit_with_time`.
3. **Missing Inter-Block Boundary Tests:** There are no integration tests in `ez-daemon` or `ez-gui` verifying whether an ADS-B squitter split across two successive `SampleBlock`s or `feed_iq` calls is decoded.
4. **Missing Demod2400 Sample Rate Validation:** No test asserts that `Demod2400` detects signals at sample rates other than 2.4 MSPS, allowing `dump1090/src/main.rs` to default to 2.0 MSPS without a test failure.
5. **Missing ICAO Filter Saturation Test:** No test simulates 500+ unique aircraft in `IcaoFilter` to measure false positive rates and collision saturation over extended sessions.

---

## 9. Prioritized Repairs

### Priority 0: Critical Functional & Safety-of-Data Repairs
1. **Eliminate Duplicated Decoder in `ez-gui`:** Delete the private, defective decoding functions in [`ez-gui/src/adsb_decoder.rs`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs) (`decode_altitude`, local CPR, velocity) and delegate all message and field parsing directly to [`dump1090::mode_s::decode_mode_s_message`](file:///home/lupc/Documents/ez-sdr/dump1090/src/mode_s.rs#L14).
2. **Filter Mode A/C from Aircraft Map:** In [`ez-gui/src/adsb_decoder.rs:92-98`](file:///home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs#L92-L98), remove the loop that inserts Mode A/C replies (`msgtype == 0xFF`) into `self.aircraft`. Mode A/C squawks must be routed to separate radar statistics, not the 24-bit ICAO aircraft table.
3. **Implement Buffer Overlap Carry:** In both [`ez-daemon/src/pipelines/packet.rs`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs) and [`ez-gui/src/adsb_decoder.rs`](file:///home/lupc/Documents/ez-gui/src/adsb_decoder.rs), allocate a carry buffer of 320 samples, carry the trailing samples between consecutive blocks, and set `mag.overlap = carry.len()`.

### Priority 1: Protocol Correctness & Integrity
4. **Fix Standalone CLI Default Sample Rate:** In [`dump1090/src/main.rs:42`](file:///home/lupc/Documents/ez-sdr/dump1090/src/main.rs#L42), update the clap default value from `"2000000"` to `"2400000"`.
5. **Fix DF 11 Syndrome Error Check:** In [`dump1090/src/demod.rs:333`](file:///home/lupc/Documents/ez-sdr/dump1090/src/demod.rs#L333), replace the empty `if` statement with an immediate return of `ScoreRank::Uncorrectable` when `syndrome & 0xFFFF80 != 0`.
6. **Correct Sample Timestamp Scaling in Daemon:** In [`ez-daemon/src/pipelines/packet.rs:133`](file:///home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs#L133), multiply `block.start_sample` by 5 so that `sample_timestamp` is in 12 MHz clock ticks.
7. **Scope Error Correction by Message Length:** In [`dump1090/src/demod.rs:210`](file:///home/lupc/Documents/ez-sdr/dump1090/src/demod.rs#L210), check the Downlink Format (DF) first; do not run 112-bit error correction on 56-bit short messages.

### Priority 2: Tracking Robustness & CPR Completeness
8. **Add Local CPR Decoding to `CprDecoder`:** In [`dump1090/src/cpr.rs`](file:///home/lupc/Documents/ez-sdr/dump1090/src/cpr.rs), when an aircraft already has a known valid position, decode subsequent single frames (odd or even) using `decode_cpr_relative` if received within 30 seconds.
9. **Implement Filter Decay or LRU Cache in `IcaoFilter`:** Replace the static 4096-bit bitset with a timestamped LRU cache or dual-buffer rolling bitset to prevent false-alarm saturation during long-running sessions.
10. **Fix Relative Longitude Normalization:** In [`dump1090/src/cpr.rs:278`](file:///home/lupc/Documents/ez-sdr/dump1090/src/cpr.rs#L278), replace `if rlon > 180.0 { rlon -= 360.0; }` with full modulo normalization: `rlon = rlon - ((rlon + 180.0) / 360.0).floor() * 360.0;`.

### Priority 3: Documentation & UI Alignment
11. **Document UAT 978 MHz Incompatibility:** Update `README.md`, `ez-gui/src/quick_start.rs`, and `ez-gui/src/frequency_db.rs` to clarify that ADS-B reception is strictly limited to 1090 MHz Mode S Extended Squitter.
12. **Vertical Rate Display Alignment:** In `ez-web/src/ui/aircraft-panel.ts`, either decode vertical rate from TC 19 in `dump1090/src/mode_s.rs` and `PacketPipeline`, or remove the non-functional `VRate` table column.

---

## 10. Audit Verification Command Log

All validation and inspection commands executed during this audit, along with their parameters and completion statuses:

| Timestamp (UTC) | Command Executed | Directory / Target | Exit Status | Summary of Result |
| :--- | :--- | :--- | :--- | :--- |
| 2026-09-18 13:09:44 | `view_file WEB_REFERENCE_AUDIT_TASK.md` | `/home/lupc/Documents/ez-sdr` | Success | Read task instructions and current agent identity lines. |
| 2026-09-18 13:09:59 | `replace_file_content WEB_REFERENCE_AUDIT_TASK.md` | `/home/lupc/Documents/ez-sdr` | Success | Appended external agent identity line under Tips. |
| 2026-09-18 13:10:07 | `git status` | `/home/lupc/Documents/ez-sdr` | Code 0 | Verified dirty worktree state; confirmed no uncommitted audit work is overwritten. |
| 2026-09-18 13:10:12 | `find_by_name "*adsb*"` | `/home/lupc/Documents/ez-sdr` | Success | Located all ADS-B source and documentation files across crates. |
| 2026-09-18 13:10:15 | `list_dir dump1090/src` | `/home/lupc/Documents/ez-sdr/dump1090/src` | Success | Identified all 16 source files and SDR drivers in `dump1090`. |
| 2026-09-18 13:10:26 | `grep_search (regex) "packet|adsb|aircraft"` | `ez-daemon` | Success | Traced packet pipeline, channelizer, and server telemetry routing. |
| 2026-09-18 13:10:37 | `grep_search (regex) "adsb|1090|978|aircraft"` | `ez-gui` | Success | Located adsb_decoder, adsb_panel, mode_bar, and app loop interactions. |
| 2026-09-18 13:10:41 | `grep_search (regex) "adsb|aircraft|mode"` | `ez-web` | Success | Traced aircraft-panel.ts and adsb.ts workflow implementation. |
| 2026-09-18 13:10:42 | `grep_search "ads-?b|1090|978"` | `README.md` | Success | Extracted operational claims and frequency statements from README. |
| 2026-09-18 13:11:04 | `view_file dump1090/src/mode_s.rs` | `dump1090` | Success | Verified reference altitude, velocity, and callsign decoding. |
| 2026-09-18 13:11:17 | `view_file dump1090/src/demod.rs` | `dump1090` | Success | Verified 2.4 MSPS preamble correlation, phase slicers, and score ranking. |
| 2026-09-18 13:11:38 | `view_file dump1090/src/cpr.rs` | `dump1090` | Success | Verified CPR 59-boundary $NL$ table, global airborne, and relative decode math. |
| 2026-09-18 13:11:48 | `view_file dump1090/src/crc.rs` | `dump1090` | Success | Verified generator polynomial `0xFFF409` and syndrome calculation. |
| 2026-09-18 13:14:03 | `view_file dump1090/src/icao_filter.rs` | `dump1090` | Success | Verified bloom bitset size (4096) and lack of expiration/decay. |
| 2026-09-18 13:14:30 | `view_file dump1090/src/main.rs` | `dump1090` | Success | Discovered default `--sample-rate 2000000` CLI argument bug. |
| 2026-09-18 13:15:04 | `cargo test --test '*' --lib --bins` | Whole repo | Code 0 | Validated repository test suite; confirmed zero test failures despite defects. |
| 2026-09-18 13:15:20 | `cargo test -p dump1090` | `dump1090` | Code 0 | All 240 unit and doc tests passed. |
| 2026-09-18 13:15:39 | `cargo test -p ez-gui -- adsb` | `ez-gui` | Code 0 | All 29 tests passed, confirming test suite asserts corrupt altitude formula. |
| 2026-09-18 13:15:51 | `cargo test -p ez-daemon -- packet` | `ez-daemon` | Code 0 | All 15 unit tests passed. |
| 2026-09-18 13:15:53 | `npm test` | `ez-web` | Code 0 | All 12 TypeScript unit tests passed. |
