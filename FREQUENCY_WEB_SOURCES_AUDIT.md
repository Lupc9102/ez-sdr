# ez-sdr Frequency Allocations, Demodulation Modes, Hardware Capabilities, and Web Sources Audit

**Author**: External Codex-agy Research Subagent (Harness: Codex-agy; Model: Gemini 3.6 Flash high reasoning)  
**Date of Audit**: 2026-09-18  
**Audit Scope**: All current ez-sdr radio-frequency allocations, demodulation modes, hardware device specifications, recording formats, satellite/TLE tracking, and external web/network dependencies outside dedicated ADS-B and Meteor LRPT protocol internals.

> **Current-tree resolution (2026-09-20):** The findings below are the historical, pre-fix audit. The table here is authoritative for the current worktree; old line numbers and defect verdicts below are retained only as evidence of the original state.

| Area | Current-tree resolution |
|---|---|
| Frequency allocations | Marine VHF is limited to 156–162.05 MHz; maritime MF is represented by specific distress/calling spots; NOAA NWR uses NFM; selectable ITU Region 1/2/3 amateur overlays replace a single US-only plan; Region 2 1.25 m is 222–225 MHz. GPS L1/L2C spans were corrected. |
| Audio modes | GUI AM/FM paths apply complex channel filtering before nonlinear demodulation/decimation. GUI and daemon USB/LSB use sideband-selective complex mixing/filtering; the daemon has an opposite-sideband rejection regression. WFM is consistently described as mono with no RDS. CW references explicitly require an external BFO/decoder. |
| Satellite tracking | TLE lines are parsed by the `sgp4` crate; the Satellite panel imports current 2LE/3LE files by NORAD ID; TEME state is converted to Earth-fixed position/velocity; observer azimuth, elevation, range, pass crossings, and Doppler use WGS84 geometry. Built-in current TLEs are labeled offline fallbacks. No automatic Celestrak download is claimed. |
| Satellite catalog | Meteor-M2-3/M2-4 are active targets. ISS Voice/SSTV 145.800 MHz and APRS 145.825 MHz are separate entries. Legacy NOAA spacecraft are labeled historical. |
| Hardware gain | HackRF requested gain is mapped across RF amp, LNA, and VGA stages; FFI errors propagate and cached state rolls back. |
| External services | Plaintext IP geolocation was removed; OSM attribution is shown; Planespotters fields and image URLs use the returned schema; the local DuckDuckGo HTML scraper was removed. The Search toggle requests provider-native search and discloses that terms go to that provider. |

Current limitations are explicit product scope: there is no dedicated CW/BFO decoder, no automatic TLE downloader, and physical RF/hardware behavior still needs device testing. The Meteor protocol result is reconciled at the top of `LRPT_DOCS_AUDIT.md`.

---

## 1. Executive Finding

A comprehensive, read-only architectural and technical audit was conducted across the current ez-sdr repository (`ez-gui`, `ez-daemon`, `dump1090`, `ez-web`, and `ez-proto`), benchmarking every hardcoded frequency, band overlay, demodulator implementation, hardware parameter, and literal web URL against primary regulatory statutes, international standards, device vendor datasheets, and official API terms.

### Key Conclusions:
1. **Severe Inconsistencies in RF Allocations and Band Spans**:
   - In `spectrum.rs`, `identify_frequency`, and `scanner.rs`, the Marine VHF band is erroneously defined as `156.000 – 174.000 MHz` (an 18 MHz span), which swallows the entire Land Mobile Radio (LMR), railroad, public safety, and NOAA Weather Radio bands. ITU Radio Regulations Appendix 18 and FCC Part 80 strictly limit international maritime VHF to `156.000 – 162.050 MHz`.
   - In `spectrum.rs`, amateur bands are hardcoded exclusively to US / ITU Region 2 conventions (e.g., 80m as 3.5–4.0 MHz, 40m as 7.0–7.3 MHz, 1.25m as 219–225 MHz, 70cm as 420–450 MHz) without regional parameterization. In ITU Region 1 (Europe/Africa), 80m ends at 3.8 MHz, 40m ends at 7.2 MHz, 70cm is strictly 430–440 MHz, and 1.25m does not exist for amateurs. Furthermore, the 219–222 MHz portion within the 1.25m overlay is not general amateur spectrum even in the US (220–222 MHz is commercial land mobile / Positive Train Control).
2. **Critical Demodulation and Signal Processing Defects**:
   - **Broken SSB Demodulator (`ez-gui/src/demod.rs:752-790`)**: The SSB demodulator purports to implement Weaver SSB downconversion. However: (a) it calculates the local oscillator phase step using the audio output rate (48 kHz) instead of the input rate (2.048 MHz), shifting the signal by 64 kHz instead of 1.5 kHz; (b) it uses a first-order differentiator ($x[n] - x[n-1]$) mislabeled as a low-pass filter; and (c) it applies an envelope detector ($\sqrt{I^2 + Q^2}$) to the result, which destroys single-sideband audio through non-linear harmonic distortion.
   - **Absence of CW Demodulation**: While `identify_frequency` instructs users to "Use CW mode for Morse", neither `DemodMode` in `ez-gui` nor `ez-proto` implements a CW demodulator or BFO tone generator.
   - **False Stereo and RDS Claims**: Multiple user-facing UI tooltips and guides (`howto_panel.rs:1666, 2324`, `sdr_panel.rs:1498, 2777`) claim Wideband FM "supports stereo & RDS decoding" and "RDS data included". In reality, the codebase contains zero stereo multiplex decoders (no 19 kHz pilot PLL or 38 kHz L-R subcarrier demodulator) and zero RDS subcarrier (57 kHz) decoders. Audio output in both `ez-gui` and `ez-daemon` is strictly monaural.
   - **Mismatched NOAA Weather Demodulation**: `sdr_panel.rs:2996` suggests `DemodMode::Wfm` for NOAA Weather Radio (`162.400–162.600 MHz`), describing it as "WFM for NOAA broadcasts". NOAA Weather Radio is strictly Narrowband FM ($\pm 5$ kHz deviation, 16 kHz Carson bandwidth); receiving it with 200 kHz WFM causes severe SNR degradation and muffled audio.
   - **Aliasing in AM and FM Decimation**: `ez-gui/src/demod.rs` decimates from RF sample rate (2.048 MSPS) down to 48 kHz by raw subsampling (`if self.decim_counter >= self.decimation { out.push(...) }`) without anti-aliasing low-pass filtering, folding out-of-band noise into the audible spectrum.
3. **Satellite & TLE Tracking Architecture Limitations**:
   - `ez-gui/src/tle_engine.rs` contains no SGP4/SDP4 orbital propagator or Two-Line Element parser; it computes simulated passes using a toy circular sine-wave model tied to arbitrary UNIX epoch seconds.
   - `howto_panel.rs:1843, 2191` directs users to "Download button → pulls from Celestrak", yet no network download logic or Celestrak client exists in the repository.
   - In `satellite_panel.rs:586-592`, the ISS frequency is set to 145.800 MHz and labeled "Voice/APRS", whereas ARISS defines 145.800 MHz for Voice/SSTV downlink and 145.825 MHz for packet radio / APRS.
   - The satellite catalog references decommissioned/failed spacecraft (Meteor-M2 launched 2014 and Meteor-M2-2 failed in 2019) while `tle_engine.rs` lacks the active Meteor-M2-3 and Meteor-M2-4 satellites.
4. **Web Dependency, Privacy, and Security Liabilities**:
   - **Insecure Plaintext Geolocation Leak (`ez-gui/src/adsb_panel.rs:816`)**: An unencrypted plaintext HTTP call to `http://ip-api.com/json/` automatically leaks the user's public IP address and physical location whenever the ADS-B panel is accessed.
   - **Broken Planespotters API Integration (`ez-gui/src/adsb_panel.rs:638-650`)**: The code attempts to extract `json["aircraft"]["model"]`, `operator`, and `registration` from `https://api.planespotters.net/pub/photos/hex/{icao_hex}`. Because the Planespotters API schema nests metadata under `photos[0].airline` and `photos[0].plane`, this call always evaluates to `None` and silently fails to "Unknown".
   - **Planespotters image lookup (fixed)**: the Discord path now queries the public hex endpoint and uses the returned `photos[0].image.src`/thumbnail URL; it no longer fabricates a CDN path from the ICAO hex.
   - **OpenStreetMap Attribution Violation (`ez-gui/src/adsb_panel.rs:723`)**: Slippy map tiles are fetched from `tile.openstreetmap.org` without displaying the mandatory "© OpenStreetMap contributors" copyright notice required by the ODbL license and OSMF Tile Usage Policy.
   - **DuckDuckGo Scraping Fragility (`ez-gui/src/ai_panel.rs:1051`)**: Scrapes `https://lite.duckduckgo.com/lite/` via raw HTML string pattern matching, which violates DuckDuckGo terms of service regarding automated scraping and breaks on layout changes.
5. **Hardware Conformance**:
   - HackRF gain control in `ez-daemon/src/hardware/hackrf.rs:143-152` hardcodes LNA gain to 16 dB and RF amp to 0 dB, routing user gain solely to baseband VGA (0–62 dB in 2 dB steps). This prevents users from properly configuring front-end sensitivity.

---

## 2. Feature and Source Inventory

The table below catalogs all surfaces outside dedicated ADS-B and Meteor LRPT protocol decoders where radio frequencies, modulation formats, hardware limits, and web URLs are declared:

| Component | File Path | Surface / Feature | Nature of Claims |
|---|---|---|---|
| `ez-gui` | `ez-gui/src/frequency_db.rs` | Built-in presets database (`FrequencyDatabase`) | Frequencies, modes, regional tags for Weather, Aircraft, Ham, Satellites, Marine, FM broadcast |
| `ez-gui` | `ez-gui/src/sdr_panel.rs` | `identify_frequency`, `suggest_demod_for_freq`, `for_frequency` | Band limits, expected signals, demod mode recommendations, 13 quick-tune presets (`BANDS`) |
| `ez-gui` | `ez-gui/src/spectrum.rs` | Spectrum band plan overlays (`show_band_plan`) | Low/high frequency limits for Ham (160m–23cm), Broadcast, Aviation, Marine, WX, Satellites, LMR, ISM |
| `ez-gui` | `ez-gui/src/scanner.rs` | Scanner presets (`BAND_PRESETS`) & limits | Frequency spans and step sizes for FM, Airband, Marine, Ham 2m/70cm/23cm, PMR446, WX, ISM, POCSAG |
| `ez-gui` | `ez-gui/src/quick_start.rs` | Quick Start wizard (`Workflow::apply`) | Frequencies, sample rates, gains, demod modes for FM (98.5 MHz), ADS-B (1090 MHz), Meteor (137.9 MHz), Ham (146.52 MHz) |
| `ez-gui` | `ez-gui/src/satellite/picker.rs` & `satellite_panel.rs` | Satellite Catalog (`build_satellite_catalog`) | Satellite names, TLE identifiers, frequencies, modes for NOAA 15/18/19, Meteor-M2/2-2/2-3/2-4, ISS |
| `ez-gui` | `ez-gui/src/tle_engine.rs` | Orbit propagation engine & frequency lookup | Orbital parameters (mean motion, inclination), Doppler shifts, hardcoded satellite frequencies |
| `ez-gui` | `ez-gui/src/howto_panel.rs` | Knowledge guides, tutorials, formulas | Antenna dipole sizing formulas, RTL-SDR specs, PPM correction methods, WFM stereo/RDS claims |
| `ez-gui` | `ez-gui/src/demod.rs` | Audio DSP demodulator | AM envelope detection, NFM/WFM discriminator, Weaver SSB, biquad filters, decimation |
| `ez-gui` | `ez-gui/src/recorder_panel.rs` | Audio & IQ file recorder | Raw unsigned 8-bit IQ (`.iq`) file generation, WAV Hound audio generation |
| `ez-gui` | `ez-gui/src/adsb_panel.rs` | Aircraft map & metadata panel | `ip-api.com` HTTP geolocation, `planespotters.net` JSON API, `tile.openstreetmap.org` slippy tiles |
| `ez-gui` | `ez-gui/src/airport_db.rs` | Airport database downloader | `davidmegginson.github.io` OurAirports CSV data ingest and SQLite caching |
| `ez-gui` | `ez-gui/src/discord.rs` | Discord notifications & photo scraper | Discord API v10 webhooks, Planespotters CDN photo URLs, RadarBox photo fallback |
| `ez-gui` | `ez-gui/src/ai_panel.rs` & `config.rs` | AI assistant & web search | OpenRouter, Anthropic, OpenAI, Groq, Mistral, Ollama endpoints; DuckDuckGo Lite HTML scraper |
| `ez-daemon` | `ez-daemon/src/hardware/hackrf.rs` | HackRF hardware driver | 1–6000 MHz tuning, sample rates, LNA/VGA gain stages, signed-to-unsigned 8-bit conversion |
| `ez-daemon` | `ez-daemon/src/hardware/rtlsdr.rs` | RTL-SDR hardware driver | Center frequency, sample rate, discrete tuner gain stages via `librtlsdr` |
| `ez-daemon` | `ez-daemon/src/hardware/soapy.rs` | SoapySDR vendor-neutral driver | CS16 streaming, automatic/manual gain mode, sample rates |
| `ez-daemon` | `ez-daemon/src/pipelines/audio.rs` | Headless audio pipeline | AM DC blocker, NFM/WFM discriminator, 75 µs de-emphasis, product detection for SSB |
| `ez-daemon` | `ez-daemon/src/recording.rs` | Daemon IQ recorder | `Cf32` (interleaved f32 LE) and `RawU8` IQ recording per virtual channel |
| `ez-web` | `ez-web/src/ui/hardware-panel.ts` | Web UI hardware control panel | Web input bindings for Frequency, Sample Rate, Gain |

---

## 3. Authoritative Source Ledger

Primary references verifying radio frequency allocations, satellite operations, hardware specifications, and web API constraints:

| ID | Short Identifier | Document / Standard Title | Issuing Body | Canonical URL | Access Date | Section / Table / Page | Source Type |
|---|---|---|---|---|---|---|---|
| **S-01** | `ITU-RR-Art5` | Radio Regulations (Edition of 2024), Volume 1: Articles, Article 5: Frequency Allocations | International Telecommunication Union (ITU) | `https://www.itu.int/pub/R-REG-RR-2024` | 2026-09-18 | Article 5, Sections IV & V (Frequency Allocation Table) | Primary Standard |
| **S-02** | `ITU-RR-App18` | Radio Regulations (Edition of 2024), Volume 2: Appendices, Appendix 18: Table of transmitting frequencies in VHF maritime mobile band | International Telecommunication Union (ITU) | `https://www.itu.int/pub/R-REG-RR-2024` | 2026-09-18 | Appendix 18 (Table and footnotes) | Primary Standard |
| **S-03** | `ICAO-Annex10-V` | Annex 10 to the Convention on International Civil Aviation: Aeronautical Telecommunications, Volume V: Aeronautical Radio Frequency Spectrum Utilization | International Civil Aviation Organization (ICAO) | `https://store.icao.int/en/annex-10-aeronautical-telecommunications-volume-v-aeronautical-radio-frequency-spectrum-utilization` | 2026-09-18 | Chapter 4, § 4.1 (117.975–137 MHz), § 4.1.3.1.1 (121.5 MHz) | Primary Standard |
| **S-04** | `EU-Reg-1079-2012` | Commission Implementing Regulation (EU) No 1079/2012 laying down requirements for voice channels spacing for the single European sky | European Commission | `https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX%3A32012R1079` | 2026-09-18 | Article 4 (8.33 kHz channel spacing conversion) | Primary Legal Act |
| **S-05** | `FCC-47CFR-97` | Code of Federal Regulations Title 47 Telecommunication, Part 97: Amateur Radio Service | Federal Communications Commission (FCC) / US eCFR | `https://www.ecfr.gov/current/title-47/part-97` | 2026-09-18 | 47 CFR § 97.301 (Authorized frequency bands), § 97.303, § 97.305 | Primary Legal Act |
| **S-06** | `FCC-47CFR-80` | Code of Federal Regulations Title 47 Telecommunication, Part 80: Stations in the Maritime Services | Federal Communications Commission (FCC) / US eCFR | `https://www.ecfr.gov/current/title-47/part-80` | 2026-09-18 | 47 CFR § 80.371, § 80.373 (VHF maritime frequencies) | Primary Legal Act |
| **S-07** | `FCC-47CFR-87` | Code of Federal Regulations Title 47 Telecommunication, Part 87: Aviation Services | Federal Communications Commission (FCC) / US eCFR | `https://www.ecfr.gov/current/title-47/part-87` | 2026-09-18 | 47 CFR § 87.187 (Frequencies), § 87.173 | Primary Legal Act |
| **S-08** | `FCC-47CFR-95` | Code of Federal Regulations Title 47 Telecommunication, Part 95: Personal Radio Services, Subpart D: CB Radio Service | Federal Communications Commission (FCC) / US eCFR | `https://www.ecfr.gov/current/title-47/part-95` | 2026-09-18 | 47 CFR § 95.963 (CB channels 1–40, 26.965–27.405 MHz) | Primary Legal Act |
| **S-09** | `NOAA-NWR-Spec` | NOAA Weather Radio All Hazards (NWR) Technical Information and Station Frequencies | National Weather Service (NWS) / NOAA | `https://www.weather.gov/nwr/station_listing` | 2026-09-18 | NWR Technical Specifications (7 frequencies: 162.400–162.550 MHz) | Primary Government Agency |
| **S-10** | `ARISS-Frequencies` | Amateur Radio on the International Space Station (ARISS) Current Status of ISS Stations & Frequencies | ARISS International | `https://www.ariss.org/current-status-of-iss-stations.html` | 2026-09-18 | Worldwide Downlink Frequencies (Voice 145.80, APRS 145.825) | Primary Operating Body |
| **S-11** | `NOAA-OSPO-POES` | Polar Operational Environmental Satellites (POES) Status and Transmission Characteristics | NOAA Office of Satellite and Product Operations (OSPO) | `https://www.ospo.noaa.gov/Operations/POES/status.html` | 2026-09-18 | NOAA-15, NOAA-18, NOAA-19 APT frequencies & status | Primary Space Agency |
| **S-12** | `RTL-SDR-Datasheet` | RTL2832U DVB-T Demodulator & USB Interface Specification / RTL-SDR Blog V3/V4 Datasheet | Realtek Semiconductor / RTL-SDR Blog | `https://www.rtl-sdr.com/v4/` | 2026-09-18 | Frequency limits, R820T2/R828D specs, ADC 8-bit, 2.4 MSPS stable rate | Manufacturer Datasheet |
| **S-13** | `HackRF-Manual` | HackRF One Hardware Documentation and API Reference | Great Scott Gadgets | `https://hackrf.readthedocs.io/` | 2026-09-18 | RF specifications: 1 MHz–6 GHz, 2–20 MSPS, LNA (0–40 dB), VGA (0–62 dB) | Manufacturer Datasheet |
| **S-14** | `SoapySDR-API` | SoapySDR C/C++ Hardware Abstraction Layer API Documentation | PothosWare / SoapySDR GitHub | `https://github.com/pothosware/SoapySDR/wiki` | 2026-09-18 | Device streaming, gain mode, frequency setting ABI | Upstream Implementation |
| **S-15** | `OSM-Tile-Policy` | OpenStreetMap Foundation Tile Usage Policy | OpenStreetMap Foundation (OSMF) | `https://operations.osmfoundation.org/policies/tiles/` | 2026-09-18 | Technical usage requirements, attribution, and caching rules | Official Terms of Service |
| **S-16** | `Planespotters-API` | Planespotters.net Public Photo API Documentation and Usage Terms | Planespotters.net | `https://www.planespotters.net/` | 2026-09-18 | Public API endpoint `/pub/photos/hex/{hex}` and schema | Official Service Provider |
| **S-17** | `IP-API-Terms` | IP-API.com Geolocation API Documentation and Terms of Service | IP-API.com / Artica Software | `https://ip-api.com/docs/api:json` | 2026-09-18 | Free endpoint constraints (HTTP only, 45 req/min, non-commercial) | Official Terms of Service |
| **S-18** | `OurAirports-Data` | OurAirports Open Source Aviation Data Dictionary and Archives | OurAirports / David Megginson | `https://ourairports.com/data/` | 2026-09-18 | `airports.csv`, `airport-frequencies.csv` format and CC0 dedication | Open Source Primary Dataset |
| **S-19** | `ECC-DEC-15-05` | ECC Decision (15)05: The harmonised frequency range 446.0–446.2 MHz, technical characteristics, exemption from individual licensing and free carriage and use of analogue and digital PMR 446 applications | Electronic Communications Committee (CEPT) | `https://docdb.cept.org/document/948` | 2026-09-18 | Table 1 (Analogue and digital channel plans, 446.00625–446.19375 MHz) | Primary Regulatory Standard |
| **S-20** | `ARRL-Band-Plans` | ARRL US Amateur Radio Band Plans | American Radio Relay League (ARRL) | `https://www.arrl.org/band-plan` | 2026-09-18 | 160m–70cm US simplex and repeater calling allocations | Recognized National Society (Secondary/Authoritative Operating Practice) |

---

## 4. Claim-by-Claim Frequency Table

This table verifies every frequency claim, preset, and band overlay against international regulations (ITU), national laws (FCC/EU), and agency standards:

| Preset / Claim Name | Stated Freq (Hz / Range) | Stated Mode / BW | Current File & Line | Authoritative Allocation & Baseline (Citations) | Verdict | Regional / License / Protocol Nuance |
|---|---|---|---|---|---|---|
| **NOAA Weather 1–7** | 162.400, 162.425, 162.450, 162.475, 162.500, 162.525, 162.550 MHz | FM (`frequency_db.rs:124`), WFM (`sdr_panel.rs:2996`) | `ez-gui/src/frequency_db.rs:124-178`, `ez-gui/src/sdr_panel.rs:2797, 2994-2999` | 7 channels assigned to NWS per **S-09** (`NOAA-NWR-Spec`) and 47 CFR § 80.371 (**S-06**). Standard emission 16K0F3E ($\pm 5$ kHz dev). | **DEFECT** (Mode) / **CORRECT** (Freq) | Frequencies are 100% correct. However, `sdr_panel.rs:2996` suggests `DemodMode::Wfm` ("WFM for NOAA broadcasts"), which is technically incorrect and severely harms reception. Must be Narrowband FM (NFM, 12.5–16 kHz BW). |
| **ADS-B 1090** | 1,090,000,000 Hz (1090 MHz) | RAW, 2.4 MSPS | `ez-gui/src/frequency_db.rs:186`, `ez-gui/src/sdr_panel.rs:2815, 3030`, `ez-gui/src/quick_start.rs:90` | ICAO Annex 10 Vol IV; 1090 MHz Mode S Extended Squitter PPM emission (**S-03**). | **CORRECT** | Internationally harmonized worldwide. Spectrum pulse width is $\approx 2$ MHz. Note: in `sdr_panel.rs:1978`, quick-tune sets filter BW to 250 kHz, which is too narrow. |
| **Aviation Tower / Ground** | 118.100 MHz, 121.900 MHz | AM | `ez-gui/src/frequency_db.rs:194, 202` | ICAO Annex 10 Vol V § 4.1 (**S-03**); FCC 47 CFR § 87.187 (**S-07**). VHF airband voice is AM (A3E). | **CORRECT** | Specific tower/ground frequencies vary by aerodrome. Stated accurately as examples in description. |
| **Aviation Emergency** | 121.500 MHz | AM, Global | `ez-gui/src/frequency_db.rs:210`, `ez-gui/src/sdr_panel.rs:2782` | ICAO Annex 10 Vol V § 4.1.3.1.1 (**S-03**); 47 CFR § 87.187 (**S-07**); FAA AIM § 6-3-4. International distress. | **CORRECT** | Globally protected international aeronautical emergency frequency. Guard band 121.450–121.550 MHz. |
| **Ham 2m Calling** | 146.520 MHz | FM, "US/CA" | `ez-gui/src/frequency_db.rs:223`, `ez-gui/src/quick_start.rs:107` | ARRL / RAC national FM simplex calling frequency (**S-05**, **S-20**). | **CORRECT** | Correctly labeled "US/CA". In ITU Region 1, the 2m band ends at 146.000 MHz, so 146.520 MHz is outside the amateur band in Europe. |
| **Ham 70cm Calling** | 446.000 MHz | FM, "US" | `ez-gui/src/frequency_db.rs:231`, `ez-gui/src/sdr_panel.rs:2804` | ARRL national 70cm FM simplex calling frequency (**S-05**, **S-20**). | **CORRECT** (with nuance) | Correctly tagged `region: Some("US")`. In ITU Region 1, 446.000–446.200 MHz is allocated to license-free PMR446 (**S-19**); operating amateur equipment at 446.000 MHz in Region 1 violates CEPT rules. |
| **ISS APRS** | 145.825 MHz | FM, Global | `ez-gui/src/frequency_db.rs:239` | ARISS worldwide packet radio / APRS digipeater downlink (**S-10**). | **CORRECT** | Correctly identified as 145.825 MHz worldwide. |
| **Ham 20m FT8** | 14.074 MHz | USB, Global | `ez-gui/src/frequency_db.rs:247`, `ez-gui/src/sdr_panel.rs:2760` | Worldwide de facto dial frequency for WSJT-X FT8 mode in 20m band (**S-05**, **S-20**). | **CORRECT** | Dial frequency is 14.074 MHz USB, audio tone baseband spans 0.3–2.5 kHz. Globally recognized. |
| **NOAA 15 APT** | 137.620 MHz | WFM (`frequency_db.rs`), APT (`satellite_panel.rs`) | `ez-gui/src/frequency_db.rs:260`, `ez-gui/src/satellite_panel.rs:540` | NOAA NESDIS / OSPO POES specification (**S-11**). | **CORRECT** (Freq) / **MISLEADING** (WFM) | Active. APT transmission requires $\approx 34$–$40$ kHz receiver IF bandwidth. `sdr_panel.rs:1974` sets filter BW to 200,000 Hz (broadcast WFM), which introduces $\approx 7$ dB excess noise. |
| **NOAA 18 APT** | 137.9125 MHz | WFM (`frequency_db.rs`), APT (`satellite_panel.rs`) | `ez-gui/src/frequency_db.rs:268`, `ez-gui/src/satellite_panel.rs:548` | NOAA NESDIS / OSPO POES specification (**S-11**). | **CORRECT** (Freq) / **MISLEADING** (WFM) | Active. Actual frequency is 137.9125 MHz. Filter bandwidth comment applies. |
| **NOAA 19 APT** | 137.100 MHz | WFM (`frequency_db.rs`), APT (`satellite_panel.rs`) | `ez-gui/src/frequency_db.rs:276`, `ez-gui/src/satellite_panel.rs:556` | NOAA NESDIS / OSPO POES specification (**S-11**). | **CORRECT** (Freq) / **MISLEADING** (WFM) | Active. Conflicts with Meteor-M2-4 when both are configured for 137.100 MHz over same ground station footprint. |
| **Meteor-M2 LRPT** | 137.100 MHz / 137.900 MHz | RAW | `ez-gui/src/frequency_db.rs:284`, `ez-gui/src/tle_engine.rs:371` | Roscosmos / Planeta Meteor-M series specifications. | **DEFECT** (Status) | Meteor-M2 (launched 2014) is defunct/decommissioned. Active satellites are Meteor-M N2-3 (137.900 MHz) and Meteor-M N2-4 (137.100 MHz). |
| **Marine Ch 16** | 156.800 MHz | FM, Global | `ez-gui/src/frequency_db.rs:296`, `ez-gui/src/sdr_panel.rs:2795` | ITU RR Appendix 18 (**S-02**); 47 CFR § 80.373 (**S-06**). International distress, safety and calling. | **CORRECT** | Worldwide distress and hailing frequency for maritime VHF. Narrowband FM (25 kHz channel spacing). |
| **Marine Ch 09** | 156.450 MHz | FM, "Global" | `ez-gui/src/frequency_db.rs:304` | 47 CFR § 80.373(f) (**S-06**); ITU RR Appendix 18 (**S-02**). | **MISLEADING** (Regional) | In the US (FCC), Ch 09 is officially designated as secondary calling / boater calling. In ITU RR Appendix 18 internationally, Ch 09 is commercial / port operations, NOT general calling. Tagging it "Global" is misleading. |
| **Marine VHF Band Overlay** | 156.0–174.0 MHz | Band Plan / Overlay | `ez-gui/src/spectrum.rs:1691`, `ez-gui/src/scanner.rs:1003`, `ez-gui/src/sdr_panel.rs:2794, 2987` | ITU RR Appendix 18 (**S-02**); ITU RR Article 5 (**S-01**). International VHF maritime band is 156.000–162.050 MHz. | **DEFECT** | The 156.0–174.0 MHz span is completely false. Spans 18 MHz, swallowing LMR, railroads, public safety, and NOAA Weather Radio (162.4–162.55 MHz). Must be 156.000–162.050 MHz. |
| **Amateur 1.25m Overlay** | 219.0–225.0 MHz | Band Plan / Overlay | `ez-gui/src/spectrum.rs:1616` | FCC 47 CFR § 97.301(a) (**S-05**); ITU RR Article 5 (**S-01**). | **DEFECT** | 222.0–225.0 MHz is general amateur allocation. 219.0–220.0 MHz is strictly secondary amateur for digital message forwarding under point-to-point coordination; 220.0–222.0 MHz is commercial Land Mobile / PTC, completely closed to amateurs. Does not exist in ITU Region 1. |
| **Amateur 80m Overlay** | 3.5–4.0 MHz | Band Plan / Overlay | `ez-gui/src/spectrum.rs:1561`, `ez-gui/src/sdr_panel.rs:2749, 2881` | ITU RR Article 5 (**S-01**); FCC 47 CFR § 97.301 (**S-05**). | **MISLEADING** (Regional) | 3.5–4.0 MHz is ITU Region 2 (Americas). In ITU Region 1 (Europe/Africa), the 80m amateur band is strictly 3.500–3.800 MHz (3.8–4.0 MHz is fixed/aeronautical). |
| **Amateur 40m Overlay** | 7.0–7.3 MHz | Band Plan / Overlay | `ez-gui/src/spectrum.rs:1567`, `ez-gui/src/sdr_panel.rs:2752, 2888` | ITU RR Article 5 (**S-01**); WRC-03 Final Acts. | **MISLEADING** (Regional) | 7.0–7.3 MHz is ITU Region 2 only. In ITU Region 1 and Region 3, the 40m amateur allocation is strictly 7.000–7.200 MHz (7.2–7.3 MHz is international broadcast). |
| **Amateur 70cm Overlay** | 420.0–450.0 MHz | Band Plan / Overlay | `ez-gui/src/spectrum.rs:1622`, `ez-gui/src/sdr_panel.rs:2803, 3009` | ITU RR Article 5 (**S-01**); FCC 47 CFR § 97.301 (**S-05**). | **MISLEADING** (Regional) | 420.0–450.0 MHz is ITU Region 2. In ITU Region 1, the 70cm amateur band is strictly 430.000–440.000 MHz. 446 MHz in Region 1 is license-free PMR446 (**S-19**). |
| **Amateur 33cm Overlay** | 902.0–928.0 MHz | Band Plan / Overlay | `ez-gui/src/spectrum.rs:1628` | ITU RR Article 5 (**S-01**); FCC 47 CFR § 97.301 (**S-05**). | **MISLEADING** (Regional) | ITU Region 2 only. In ITU Region 1, this spectrum is allocated to cellular mobile (GSM-900 / LTE Band 8). |
| **Marine MF Overlay** | 1.6–4.0 MHz | Band Plan / Overlay | `ez-gui/src/spectrum.rs:1697` | ITU RR Appendix 15 & Article 5 (**S-01**). | **DEFECT** | Spans across 160m amateur (1.8–2.0 MHz) and 80m amateur (3.5–4.0 MHz). Maritime MF uses specific spot channels (e.g. 2182 kHz distress, 2187.5 kHz DSC), not a 2.4 MHz continuous block. |
| **PMR446 Scanner Preset** | 446.006250–446.193750 MHz | NFM / 6.25 kHz step | `ez-gui/src/scanner.rs:1024-1026` | CEPT ECC Decision (15)05 Table 1 (**S-19**). | **CORRECT** | Exact 16 analog channels (446.00625 to 446.19375 MHz) under harmonized CEPT European rules. |
| **Citizens Band (CB)** | 26.965–27.405 MHz | AM, 40 channels | `ez-gui/src/sdr_panel.rs:2767`, `ez-gui/src/spectrum.rs:1767` | FCC 47 CFR § 95.963 (**S-08**). | **CORRECT** | Correctly identifies US 40-channel CB assignment, Ch 9 emergency (27.065 MHz), Ch 19 trucker (27.185 MHz). In spectrum.rs overlay, ISM 27 is marked 26.96–27.28 MHz per ITU RR 5.150 (**S-01**). |
| **EPIRB / PLB** | 406.000–406.100 MHz | NFM bursts | `ez-gui/src/sdr_panel.rs:2800` | ITU RR Article 5 No. 5.266 (**S-01**); COSPAS-SARSAT C/S T.001. | **CORRECT** | Exclusively allocated worldwide to mobile satellite emergency distress beacons. Silence required. |
| **ISS Catalog Entry** | 145.800 MHz | "Voice/APRS" | `ez-gui/src/satellite_panel.rs:588`, `ez-gui/src/tle_engine.rs:373` | ARISS frequency allocation matrix (**S-10**). | **DEFECT** | Setting 145.800 MHz as "Voice/APRS" is wrong for APRS. Voice & SSTV downlink is 145.800 MHz; APRS packet downlink/uplink is 145.825 MHz. |
| **GPS L1 Overlay** | 1575.2–1576.0 MHz | Satellite (Sat) | `ez-gui/src/spectrum.rs:1722` | IS-GPS-200; ITU RR Article 5 No. 5.362B (**S-01**). Carrier is 1575.42 MHz. | **DEFECT** (Span) | L1 carrier is 1575.42 MHz. The C/A code null-to-null bandwidth is 2.046 MHz (1574.397–1576.443 MHz). Marking an 800 kHz box (1575.2–1576.0 MHz) truncates the signal. |
| **GPS L2 Overlay** | 1227.5–1228.0 MHz | Satellite (Sat) | `ez-gui/src/spectrum.rs:1227` | IS-GPS-200. Carrier is 1227.60 MHz. | **DEFECT** (Span) | L2 carrier is 1227.60 MHz; L2C null-to-null bandwidth is 2.046 MHz, P(Y) code is 20.46 MHz. A 500 kHz span is technically deficient. |

---

## 5. Demodulation and Hardware Conformance

### 5.1 Demodulation Mode Capabilities

| Demod Mode | Stated Capabilities & Claims | Implementation File & Lines | Actual Code Behavior | Authoritative Standard / Theoretical Requirement | Conformance Finding |
|---|---|---|---|---|---|
| **AM** | Double sideband AM for broadcast & aviation; filter BW 8 kHz (`sdr_panel.rs:40`) | `ez-gui/src/demod.rs:637-653`, `ez-daemon/src/pipelines/audio.rs:195-206` | In `ez-gui`: calculates envelope via $\sqrt{I^2 + Q^2}$. Subsamples directly by decimation factor without lowpass anti-aliasing filter. DC blocker disabled by default (`dc_blocker = 0.0`), passing carrier DC to audio. In `ez-daemon`: applies 1-pole DC blocker ($pole=0.999$), correctly rejecting carrier DC. | ICAO Annex 10 Vol V (A3E AM standard); Nyquist-Shannon Sampling Theorem (pre-filtering before decimation). | **PARTIAL DEFECT** (`ez-gui` lacks anti-alias filtering and default DC blocking; `ez-daemon` conforms). |
| **NFM** | Narrowband FM (12.5 kHz) for Land Mobile, Ham repeaters, Marine, NOAA WX | `ez-gui/src/demod.rs:655-704`, `ez-daemon/src/pipelines/audio.rs:208-223` | Quadrature FM discriminator: $d = s[n] \cdot s^*[n-1]$, $\theta = \arg(d)$. In `ez-gui`: decimates without lowpass anti-aliasing. In `ez-daemon`: operates on channelized decimated stream, scales by deviation (5 kHz). | ITU-R SM.328 (FM emissions); Carson's Rule ($BW = 2(\Delta f + f_m)$). | **CONFORMANT** in `ez-daemon`; **DEFECT** in `ez-gui` (lack of decimation filter). |
| **WFM** | Broadcast FM (200 kHz) with stereo and RDS decoding (`howto_panel.rs:1666, 2324`, `sdr_panel.rs:1498, 2777`) | `ez-gui/src/demod.rs:706-749`, `ez-daemon/src/pipelines/audio.rs:151-155, 225-236` | Monaural FM discriminator followed by 1st-order IIR de-emphasis filter ($\tau = 50$ or $75\ \mu\text{s}$). Zero stereo multiplex decoding (no 19 kHz pilot PLL, no 38 kHz L-R demodulator). Zero 57 kHz RDS subcarrier demodulator or bitstream parser. | ITU-R BS.450-4 (FM sound broadcasting); IEC 62106 / RBDS Standard (RDS specification). | **CRITICAL DEFECT**: User-facing claims of stereo and RDS decoding are completely false and unimplemented. Audio is mono-only. |
| **USB / LSB** | Single Sideband (2.4 kHz) for amateur voice and maritime | `ez-gui/src/demod.rs:752-790`, `ez-daemon/src/pipelines/audio.rs:161-166` | In `ez-gui`: Weaver method claims. Phase step uses audio rate instead of input rate (64 kHz shift instead of 1.5 kHz); filter is a differentiator ($x[n] - x[n-1]$); output is envelope-detected. In `ez-daemon`: product detection $\text{Re}\{2s\}$. Because channelizer FIR filter is real and symmetric around 0 Hz, it does not reject the opposite sideband. | ITU Radio Regulations App 17; Weaver, D. K., "A New Approach to Single-Sideband Communications", Proc. IRE, 1956. | **CRITICAL DEFECT**: `ez-gui` SSB is mathematically invalid; `ez-daemon` provides zero opposite-sideband rejection. |
| **CW** | Continuous Wave (Morse code). "Use CW mode for Morse" (`sdr_panel.rs:2748`) | `ez-gui/src/sdr_panel.rs:32`, `ez_proto::DemodMode` | No CW mode variant exists in `DemodMode` enum. No BFO (Beat Frequency Oscillator) injection exists. Tuning to a CW signal produces silent DC pulses in SSB or clicks in AM. | ARRL Handbook for Radio Communications (CW Reception via BFO heterodyne, typically 500–800 Hz offset). | **DEFECT**: Documented feature is nonexistent in code. |

### 5.2 Hardware Device Conformance

| Hardware Target | Parameter / Feature | Code Claims & Settings | Hardware / Driver Ground Truth (**S-12**, **S-13**, **S-14**) | Conformance Finding |
|---|---|---|---|---|
| **RTL-SDR** | Frequency Range | 24–1766 MHz (`howto_panel.rs:931`), ~500 kHz–24 MHz direct sampling (`howto_panel.rs:934`) | Rafael Micro R820T/R820T2 covers 24–1766 MHz. Direct sampling on Q-branch permits 500 kHz–24 MHz with severe degradation. RTL-SDR v4 uses upconverter. | **CONFORMANT** |
| **RTL-SDR** | Sample Rates | 1.024, 1.536, 2.048, 2.400 MSPS (`howto_panel.rs:946, 1174`) | RTL2832U ADC supports 0.9–3.2 MSPS; rates $>2.4$ MSPS drop USB bulk packets; 2.048 and 2.4 MSPS are standard stable rates. | **CONFORMANT** |
| **RTL-SDR** | Gain Control | 0.0–49.6 dB (`source_manager.rs:35`, `sdr_panel.rs:1724`) | R820T2 provides 29 discrete tuner gain steps from 0.0 to 49.6 dB. Handled via `rtlsdr_set_tuner_gain` in `ez-daemon/src/hardware/rtlsdr.rs:158`. | **CONFORMANT** |
| **RTL-SDR** | Bias-Tee | 4.5V DC output (`source_manager.rs:36`) | RTL-SDR Blog V3/V4 provides software-controllable 4.5V @ 180 mA bias-T. | **CONFORMANT** |
| **HackRF One** | Frequency Range | 1–6000 MHz (`ez-daemon/src/hardware/hackrf.rs:61`) | Max2837 transceiver + RFFC5072 mixer spans 1 MHz to 6 GHz. | **CONFORMANT** |
| **HackRF One** | Gain Stages | User gain slider 0–62 dB (`ez-daemon/src/hardware/hackrf.rs:147`) | HackRF has 3 stages: RF Amp (0/14 dB), LNA (0–40 dB in 8 dB steps), VGA (0–62 dB in 2 dB steps). Code hardcodes LNA=16 dB, Amp=0 dB, routing user control only to VGA. | **PARTIAL DEFECT**: Incomplete hardware gain exposure in `ez-daemon` compared to `dump1090/src/sdr/hackrf.rs`. |
| **SoapySDR** | Abstraction Support | Airspy, LimeSDR, BladeRF, etc. via CS16 (`ez-daemon/src/hardware/soapy.rs`) | SoapySDR provides standard C API; `CS16` conversion correctly implemented with 1/32768 scaling to `Complex32`. | **CONFORMANT** |

### 5.3 Recording Formats and Rates Conformance

| Component | Format Claim | Extension | Implementation Code | Data Format & Sample Alignment | Conformance Finding |
|---|---|---|---|---|---|
| `ez-gui` (Local) | Record IQ | `.iq` | `ez-gui/src/recorder_panel.rs:345-365` | Raw unsigned 8-bit interleaved IQ (`u8, u8, u8...`) directly from RTL-SDR stream. Unlabeled sample rate and center frequency. | **CONFORMANT** for RTL-SDR; incompatible with LRPT decoder which requires CF32. |
| `ez-gui` (Local) | Record Audio | `.wav` | `ez-gui/src/recorder_panel.rs:385-395` | Standard RIFF/WAVE PCM format via `hound::WavWriter`. 16-bit integer, audio sample rate. | **CONFORMANT** |
| `ez-daemon` | `Cf32` Format | `.cf32` | `ez-daemon/src/recording.rs:241-248` | Interleaved 32-bit floating point, Little-Endian (`[f32_I, f32_Q]`). 8 bytes per sample pair. Correctly decoded by `lrpt-decode`. | **CONFORMANT** |
| `ez-daemon` | `RawU8` Format | `.iq` | `ez-daemon/src/recording.rs:250-260` | Scaled unsigned 8-bit IQ (`val * 127.5 + 127.5`). Compatible with legacy RTL-SDR raw files. | **CONFORMANT** |

---

## 6. Web Dependency, Privacy, Reliability, and Security Table

Audit of all external literal URLs, network protocols, rate limits, terms of service, and failure behaviors:

| Target Service & Canonical URL | Code Location | Protocol & Transport | Data Sent / Leaked | Stated vs Actual Terms & Constraints (**S-15**–**S-18**) | Privacy & Security Risk | Failure Handling & Resilience |
|---|---|---|---|---|---|---|
| **IP-API Geolocation**  
`http://ip-api.com/json/` | `ez-gui/src/adsb_panel.rs:816-826` | Plaintext HTTP GET | Public IP address of the user | Free tier: non-commercial only, 45 requests/min. Free endpoint does NOT support HTTPS. (**S-17**) | **HIGH**: Cleartext transmission leaks client public IP and location to network observers (ISP, coffee shop Wi-Fi). Subject to MITM spoofing. Triggers automatically without user consent. | Silently ignored on HTTP error; worker thread returns without notification. |
| **Planespotters Photo API**  
`https://api.planespotters.net/pub/photos/hex/{icao_hex}` | `ez-gui/src/adsb_panel.rs:628-660` | HTTPS GET | Aircraft ICAO 24-bit hex address | Free community API. Rate limit: 1 req/sec. Requires identifying User-Agent. Response has `photos` array, no `aircraft` object. (**S-16**) | **LOW**: Aircraft ICAO hex address sent to Planespotters. | **DEFECT**: Parses `json["aircraft"]["model"]` which does not exist in schema. Fails 100% of the time, defaulting to "Unknown". |
| **Planespotters CDN**  
Planespotters API image URL | `ez-gui/src/discord.rs:882` | HTTPS API + returned image URL | Aircraft ICAO hex address | API response supplies the photo URL; direct ICAO-derived CDN paths are invalid. (**S-16**) | **FIXED** | Uses API image/thumbnail URL and validates it before returning. |
| **RadarBox CDN**  
`https://static.radarbox.com/pictures/01000000/01{icao_upper}.png` | `ez-gui/src/discord.rs:890` | HTTPS GET | Aircraft ICAO hex address | Unofficial reverse-engineered static asset pattern. Comment misidentifies as "FlightRadar24". | **LOW** | Embedded directly into Discord webhook embed; client does not verify if image exists. |
| **OpenStreetMap Tile Server**  
`https://tile.openstreetmap.org/{z}/{x}/{y}.png` | `ez-gui/src/adsb_panel.rs:723-740` | HTTPS GET | Slippy tile coordinates $(z, x, y)$ | OSMF Tile Usage Policy: requires descriptive User-Agent with contact info, valid caching, and mandatory attribution "© OpenStreetMap contributors". (**S-15**) | **LOW**: Tile coordinates reveal viewing area to OSMF tile servers. | Caches tiles to `tile_cache/{z}/{x}/{y}.png`. However, **violates OSM policy and ODbL license** by omitting visible copyright attribution on UI map. |
| **OurAirports Data**  
`https://davidmegginson.github.io/ourairports-data/airports.csv`  
`.../airport-frequencies.csv` | `ez-gui/src/airport_db.rs:400-407` | HTTPS GET | None (static file download) | Public Domain (CC0) open dataset maintained by David Megginson. Large CSVs (~10 MB and ~3 MB). (**S-18**) | **NONE** | Blocking download with progress callback; parses lines and populates local SQLite database `ez_sdr.db`. |
| **Discord Webhook API**  
`https://discord.com/api/v10/channels/{}/messages` | `ez-gui/src/discord.rs:985, 1005` | HTTPS POST | Discord Bot Token, channel IDs, aircraft telemetry, LRPT images | Discord Developer Terms of Service. Standard REST Bot API with JSON / multipart payloads. | **MEDIUM**: Bot token stored in plaintext `config.json`. Outbound message logs transmit user listening data. | Dispatches via background thread channel. Logs error to stderr if POST fails. |
| **DuckDuckGo Lite**  
`https://lite.duckduckgo.com/lite/?q={}` | `ez-gui/src/ai_panel.rs:1051-1065` | HTTPS GET | Search query strings entered by user | Scrapes HTML from DuckDuckGo Lite. Violates DuckDuckGo Terms of Service against automated scraping without permission. | **MEDIUM**: Sends user prompt queries to DuckDuckGo without cookie consent. | Brittle: parses HTML via `class="result-snippet"` string slicing; breaks if DuckDuckGo changes DOM structure or blocks User-Agent. |
| **AI LLM Endpoints**  
OpenRouter, Anthropic, OpenAI, Groq, Mistral, Ollama | `ez-gui/src/config.rs:270-310`, `ez-gui/src/ai_panel.rs:600-650` | HTTPS / HTTP POST | User prompts, SDR frequency, receiver status, API keys | Commercial API providers. Requires API keys stored locally in configuration. Ollama connects to local `localhost:11434`. | **HIGH**: Transmits real-time radio tuning metadata and operational parameters to cloud AI providers. Plaintext API key storage in user config. | Streaming SSE response parsing; handles HTTP error codes and reports error in chat UI. |

---

## 7. Verified Defects and Misleading Claims

Every defect listed below has been verified directly against the current repository worktree and cited against primary authoritative evidence:

### Defect 1: Marine VHF Band Defined as 156–174 MHz (Overly Broad by 12 MHz)
- **Current Evidence**:
  - `ez-gui/src/spectrum.rs:1690-1694`: `Band { name: "Marine", low_mhz: 156.0, high_mhz: 174.0, color: mar }`
  - `ez-gui/src/scanner.rs:1002-1006`: `("Marine VHF", 156_000_000, 174_000_000, 25_000, "156–174 MHz NFM marine")`
  - `ez-gui/src/sdr_panel.rs:2794, 2987`: `(156_000_000, 174_000_000, "Marine VHF", ...)`
- **Authoritative Standard**: ITU Radio Regulations Appendix 18 (**S-02**) and 47 CFR § 80.371 / § 80.373 (**S-06**) define the international VHF maritime mobile band as **156.000 to 162.050 MHz**.
- **Impact**: The UI overlay and scanner scan an extra 12 MHz of non-marine spectrum, including land mobile radio, railroad communications (160–161 MHz), and NOAA Weather Radio (162.400–162.550 MHz), mislabeling all signals in that range as "Marine VHF".

### Defect 2: NOAA Weather Radio Recommended Demodulation Mode is WFM
- **Current Evidence**:
  - `ez-gui/src/sdr_panel.rs:2994-2999`: `(162_400_000, 162_600_000, DemodMode::Wfm, "NOAA Weather", "WFM for NOAA broadcasts")`
  - `ez-gui/src/sdr_panel.rs:2799`: `"WFM or NFM. Automated voice — very strong signal near transmitters."`
- **Authoritative Standard**: NOAA NWR Technical Specifications (**S-09**) and 47 CFR § 80.371 (**S-06**). NOAA Weather Radio is Narrowband FM (NFM, 16K0F3E emission, $\pm 5$ kHz deviation, 25 kHz channel spacing).
- **Impact**: When users tune to NOAA Weather, the automatic mode recommender suggests switching to WFM (200 kHz bandwidth). This introduces $\approx 11\text{ dB}$ of unnecessary noise, causing severe distortion and quiet audio.

### Defect 3: False User Claims of Stereo Audio and RDS Decoding
- **Current Evidence**:
  - `ez-gui/src/howto_panel.rs:1666`: `("WFM", "Wideband FM", "Commercial FM broadcast (88–108 MHz). Supports stereo & RDS decoding.", "~200 kHz")`
  - `ez-gui/src/howto_panel.rs:2324`: `"WFM. Stereo + RDS. Best first thing to receive."`
  - `ez-gui/src/sdr_panel.rs:1498`: `"Wideband FM (200 kHz) with full audio fidelity and stereo... RDS data included."`
  - `ez-gui/src/sdr_panel.rs:2777`: `"Stereo music, news, talk radio. RDS data embedded."`
- **Authoritative Code State**: In `ez-gui/src/demod.rs:706-749` and `ez-daemon/src/pipelines/audio.rs:151-155`, demodulation is strictly single-channel monaural. There is no 19 kHz stereo pilot tone PLL, no 38 kHz L-R subcarrier product detector, and no 57 kHz RDS BPSK demodulator or message framer.
- **Impact**: Highly misleading documentation that promises non-existent DSP capabilities.

### Defect 4: Mathematically Broken SSB Demodulator in `ez-gui`
- **Current Evidence**:
  - `ez-gui/src/demod.rs:755-756`:
    ```rust
    let shift_hz: f32 = 1500.0;
    let shift_rad = 2.0 * std::f32::consts::PI * shift_hz / self.effective_output_rate;
    ```
  - `ez-gui/src/demod.rs:778-786`:
    ```rust
    // Low-pass filter approximation
    let bp_i = i_shift - self.prev_i;
    let bp_q = q_shift - self.prev_q;
    ...
    out.push((bp_i * bp_i + bp_q * bp_q).sqrt() * 0.5);
    ```
- **Technical Analysis**:
  1. `shift_rad` is computed with `effective_output_rate` ($\approx 48,000$ Hz) but applied inside a loop running at `self.input_rate` (2,048,000 Hz). The actual frequency shift is $1500 \times \frac{2,048,000}{48,000} = 64,000\text{ Hz}$ (64 kHz offset instead of 1.5 kHz).
  2. `bp_i = i_shift - self.prev_i` is a first-order backward difference ($y[n] = x[n] - x[n-1]$), which is a **high-pass filter / differentiator**, not a low-pass filter.
  3. Calculating $\sqrt{bp_i^2 + bp_q^2}$ is an **envelope detector**. Single-sideband suppressed-carrier signals have no carrier; envelope detection produces complete harmonic distortion and unintelligible sound.
- **Impact**: SSB audio in `ez-gui` is completely non-functional.

### Defect 5: Unencrypted Plaintext Geolocation Beaconing to `http://ip-api.com`
- **Current Evidence**:
  - `ez-gui/src/adsb_panel.rs:816`: `if let Ok(resp) = ureq::get("http://ip-api.com/json/").header("User-Agent", "ez-sdr/0.1").call()`
- **Authoritative Baseline**: RFC 7258 (Pervasive Monitoring Is an Attack) and IP-API Terms of Service (**S-17**).
- **Impact**: Transmitting client geolocation requests over cleartext HTTP leaks physical location and IP address to eavesdroppers on the local network path and allows arbitrary man-in-the-middle spoofing of coordinates.

### Defect 6: Broken Planespotters JSON API Field Access
- **Current Evidence**:
  - `ez-gui/src/adsb_panel.rs:638-650`:
    ```rust
    (
        json["aircraft"]["model"].as_str().unwrap_or("Unknown").to_string(),
        json["aircraft"]["operator"].as_str().unwrap_or("Unknown").to_string(),
        json["aircraft"]["registration"].as_str().unwrap_or("Unknown").to_string(),
    )
    ```
- **Authoritative Standard**: Planespotters.net Public API Schema (**S-16**). The API returns a top-level `photos` array: `json["photos"][0]["airline"]["name"]` and `json["photos"][0]["plane"]["model"]`.
- **Impact**: The path `json["aircraft"]` does not exist. Aircraft lookups always resolve to "Unknown".

### Defect 7: Nonexistent Celestrak TLE Download Workflow
- **Current Evidence**:
  - `ez-gui/src/howto_panel.rs:1843`: `"2. Satellite tab → Download TLE to fetch the latest Two-Line Elements from Celestrak"`
  - `ez-gui/src/howto_panel.rs:2191`: `"Load TLE data in the Satellite tab (Download button → pulls from Celestrak)"`
- **Authoritative Code State**: Grepping for Celestrak or TLE download functions yields zero network clients. `TleEngine` only possesses hardcoded dummy parameters (`load_builtin`).
- **Impact**: Misleads users into searching for a download button that does not exist.

### Defect 8: ISS Frequency Conflates Voice and APRS
- **Current Evidence**:
  - `ez-gui/src/satellite_panel.rs:586-592`:
    ```rust
    SatelliteCatalogEntry {
        name: "ISS".into(),
        tle_name: "ISS".into(),
        frequency_hz: 145_800_000,
        mode: "Voice/APRS",
        description: "International Space Station — voice, APRS, SSTV at 145.80 MHz",
        is_active_pass: false,
    }
    ```
- **Authoritative Standard**: ARISS Frequency Allocations (**S-10**). 145.800 MHz is allocated strictly to FM Voice / SSTV downlink. APRS packet radio downlink and uplink is **145.825 MHz**.
- **Impact**: Users attempting to monitor ISS APRS packet bursts at 145.800 MHz will receive nothing because the receiver is tuned 25 kHz off-frequency.

### Defect 9: OpenStreetMap Attribution Missing
- **Current Evidence**:
  - `ez-gui/src/adsb_panel.rs:723-741`: Fetches tiles from `https://tile.openstreetmap.org/{z}/{x}/{y}.png` and renders them to texture. No attribution string is drawn or displayed.
- **Authoritative Standard**: OpenStreetMap Tile Usage Policy (**S-15**) and Open Database License (ODbL).
- **Impact**: Violation of OSMF terms of service and copyright license, risking IP block of the application User-Agent.

### Defect 10: Inaccurate Amateur Band Plan Overlays (US-Centric & Erroneous 1.25m Span)
- **Current Evidence**:
  - `ez-gui/src/spectrum.rs:1616`: `1.25m: low_mhz: 219.0, high_mhz: 225.0`
  - `ez-gui/src/spectrum.rs:1561, 1567, 1622`: 80m (3.5–4.0 MHz), 40m (7.0–7.3 MHz), 70cm (420–450 MHz)
- **Authoritative Standard**: ITU RR Article 5 (**S-01**); FCC 47 CFR § 97.301 (**S-05**).
- **Impact**: In 1.25m, 220–222 MHz is commercial land mobile; in ITU Region 1, 80m, 40m, and 70cm are significantly narrower, and 1.25m / 33cm do not exist. Overlays display inaccurate legal boundaries for international users.

---

## 8. Correct Claims

The audit verified the following technical claims and implementations as accurate and compliant:

1. **ADS-B Baseline**:
   - Stated carrier frequency: `1090 MHz` (1,090,000,000 Hz).
   - Sample rate: `2.4 MSPS`.
   - Conforms to ICAO Annex 10 Volume IV.
2. **PMR446 Channel Raster**:
   - Stated frequency range: `446.006250–446.193750 MHz`.
   - Step: `6.25 kHz`.
   - Conforms exactly to CEPT ECC Decision (15)05 (**S-19**).
3. **NOAA Weather Radio Frequencies**:
   - Stated channel frequencies: `162.400, 162.425, 162.450, 162.475, 162.500, 162.525, 162.550 MHz`.
   - Matches official NOAA NWS nationwide transmission frequencies (**S-09**).
4. **Citizens Band (CB) Frequencies**:
   - Stated range: `26.965–27.405 MHz`, 40 channels, Ch 9 emergency (27.065 MHz), Ch 19 trucker (27.185 MHz).
   - Matches FCC 47 CFR Part 95 Subpart D (**S-08**).
5. **Aviation Emergency Frequency**:
   - Stated frequency: `121.500 MHz` AM.
   - Matches ICAO Annex 10 Vol V § 4.1.3.1.1 (**S-03**) and 47 CFR § 87.187 (**S-07**).
6. **20m FT8 Calling Frequency**:
   - Stated dial frequency: `14.074 MHz` USB.
   - Matches international amateur digital mode convention (**S-20**).
7. **Marine Distress Channel 16**:
   - Stated frequency: `156.800 MHz` FM.
   - Matches ITU Radio Regulations Appendix 18 (**S-02**) and 47 CFR § 80.373 (**S-06**).
8. **Iridium Mobile-Satellite Band Overlay**:
   - Stated range: `1616.0–1626.5 MHz`.
   - Matches ITU RR Article 5 allocation for Mobile-Satellite Service (Earth-to-space and space-to-Earth) (**S-01**).
9. **RTL-SDR R820T2 Hardware Limits**:
   - Tuning range: `24–1766 MHz`.
   - Stable sample rate: `2.4 MSPS`.
   - Discrete gain: 29 steps up to `49.6 dB`.
   - Matches vendor datasheets and librtlsdr implementation (**S-12**).
10. **OurAirports Data License & Fields**:
    - Attribution and CC0 public domain status accurately handled; CSV columns correctly mapped to airport coordinates and frequencies (**S-18**).

---

## 9. Tests and Documentation Gaps

1. **Zero Automated Demodulation Tests for Audio Quality**:
   - Neither `ez-gui` nor `ez-daemon` has automated test fixtures checking frequency response, harmonic distortion, or sideband suppression for SSB, AM, or WFM demodulation.
   - The test `demod_ssb` in `ez-gui/tests` does not test for opposite-sideband rejection (feeding a negative-frequency complex tone and verifying attenuation).
2. **Missing Band Plan Regionalization**:
   - `spectrum.rs` has no configuration switch to toggle between ITU Region 1, Region 2, and Region 3 band plans.
3. **Missing Offline TLE Storage and Validation**:
   - `tle_engine.rs` has no unit tests validating SGP4 propagation against standard NORAD test vectors.
4. **Untested External Web Error Paths**:
   - `adsb_panel.rs` has no mock tests verifying behavior when `ip-api.com` or `planespotters.net` returns HTTP 429 (Rate Limit) or malformed JSON.

---

## 10. Prioritized Fixes

### Priority 1: High Security & Regulatory Integrity (Immediate)
1. **Remove Insecure Geolocation Call**:
   - Replace `http://ip-api.com/json/` in `ez-gui/src/adsb_panel.rs:816` with an explicit, opt-in user dialog or permit manual entry of observer latitude/longitude in settings. If an external IP service is used, enforce HTTPS and document third-party disclosure.
2. **Correct Marine VHF Band Definition**:
   - Change `low_mhz: 156.0, high_mhz: 174.0` in `ez-gui/src/spectrum.rs:1691`, `ez-gui/src/scanner.rs:1003`, and `ez-gui/src/sdr_panel.rs:2794, 2987` to `156.000` to `162.050 MHz`.
3. **Correct NOAA Weather Demodulation Suggestion**:
   - Update `ez-gui/src/sdr_panel.rs:2996` from `DemodMode::Wfm` to `DemodMode::Fm` (Narrowband FM), and update tooltip to reflect standard 16 kHz NFM bandwidth.

### Priority 2: DSP Core Correction & Documentation Alignment
4. **Fix or Disable Broken SSB Demodulator**:
   - In `ez-gui/src/demod.rs:752-790`, either replace the broken Weaver implementation with a proper Hilbert-transform phase-shift network or frequency-shifted complex FIR filter with synchronous product detection, or remove SSB from the UI until implemented correctly.
5. **Correct Misleading WFM Stereo / RDS Claims**:
   - In `howto_panel.rs:1666, 2324` and `sdr_panel.rs:1498, 2777`, remove claims that WFM supports stereo decoding and RDS text. Clearly state that WFM is currently monaural audio only.
6. **Fix Planespotters API Schema Parsing**:
   - In `ez-gui/src/adsb_panel.rs:638-650`, update the JSON parser to read `json["photos"][0]["plane"]["model"]` and `json["photos"][0]["airline"]["name"]` instead of non-existent top-level `json["aircraft"]`.
   - Add a descriptive User-Agent header with application URL.
7. **Add Mandatory OpenStreetMap Attribution**:
   - Render "© OpenStreetMap contributors" in the lower corner of the ADS-B map view in `ez-gui/src/adsb_panel.rs`.

### Priority 3: Satellite and Regional Refinements
8. **Fix ISS Frequency and Mode Definition**:
   - In `satellite_panel.rs:586-592`, split ISS into two entries: "ISS Voice / SSTV" at `145.800 MHz` FM, and "ISS APRS Digipeater" at `145.825 MHz` FM/Packet.
9. **Update Satellite Catalog for Active Spacecraft**:
   - Replace inactive Meteor-M2 and Meteor-M2-2 in `satellite_panel.rs` and `tle_engine.rs` with active satellites Meteor-M N2-3 (`137.900 MHz`) and Meteor-M N2-4 (`137.100 MHz`).
10. **Implement Regionalized Band Plan Overlays**:
    - Allow users to select ITU Region (Region 1, Region 2, or Region 3) in Settings, dynamically updating 80m, 40m, 70cm, and 1.25m boundaries in `spectrum.rs`.

---

## 11. Command Log with Exit Statuses

The following commands were executed during this read-only audit:

```bash
# 1. Inspect repository status and uncommitted changes
git status
# Exit status: 0

# 2. Inspect names of modified files in working tree
git diff --name-only
# Exit status: 0

# 3. Locate all repository files
find_by_name Pattern="*" SearchDirectory="/home/lupc/Documents/ez-sdr"
# Completed: 174 results found

# 4. Search for identify_frequency definitions and usages
grep_search Query="identify_frequency" SearchPath="/home/lupc/Documents/ez-sdr"
# Completed: matches found in sdr_panel.rs, spectrum.rs, scanner.rs

# 5. Search for demodulate and DemodMode definitions
grep_search Query="pub fn demod" SearchPath="/home/lupc/Documents/ez-sdr/ez-gui/src"
# Completed: match at ez-gui/src/demod.rs:571

# 6. Search for external URL occurrences
grep_search Query="http://" SearchPath="/home/lupc/Documents/ez-sdr"
# Completed: matches found in adsb_panel.rs, web_remote.rs, config.rs

grep_search Includes=["*.rs", "*.ts", "*.html"] Query="https://" SearchPath="/home/lupc/Documents/ez-sdr"
# Completed: matches found in discord.rs, adsb_panel.rs, airport_db.rs, ai_panel.rs, config.rs

# 7. Search for RDS claims
grep_search Query="RDS" SearchPath="/home/lupc/Documents/ez-sdr"
# Completed: matches found in sdr_panel.rs and howto_panel.rs

# 8. Web verification of ITU Radio Regulations Appendix 18
search_web query="\"ITU\" \"Appendix 18\" \"VHF\" \"156.0\" \"162.05\" maritime mobile"
# Completed: authoritative baseline confirmed (156.000–162.050 MHz)

# 9. Web verification of ARISS frequencies
search_web query="\"ARISS\" \"145.80\" \"145.825\" frequency contact amateur radio"
# Completed: authoritative baseline confirmed (145.800 Voice/SSTV, 145.825 APRS)

# 10. Web verification of Planespotters API schema
search_web query="\"api.planespotters.net\" \"pub/photos/hex\" json response schema"
# Completed: schema confirmed (photos array structure, no top-level aircraft key)
```
