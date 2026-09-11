# EZ-SDR Unified

A cross-platform SDR application combining a real-time spectrum analyser/waterfall, multiple demodulation modes, satellite tracking, ADS‑B decoding, and audio recording — all in a single GPU‑accelerated GUI powered by `egui`/`eframe`.

## Features

- **Source agnostic** — works with any SoapySDR‑compatible device (RTL‑SDR, HackRF, Airspy, LimeSDR, …) and file/network IQ inputs
- **Spectrum analyser** — pan/zoom FFT with configurable FFT size (256‑32768), window type (Blackman‑Harris Nuttall, Hamming, Hann, Kaiser, …), and averaging (exponential moving average α slider)
- **Waterfall** — scrolling spectrogram with adjustable speed (1×/2×/4×/8×) and per‑pixel interpolation
- **Band plan overlay** — amateur radio band edges (160m‑70cm) displayed as coloured vertical strips on the spectrum
- **Click‑to‑tune** — left‑click the spectrum plot to set the VFO frequency instantly
- **Demodulators** — RAW, AM, FM/NFM, WFM, LSB, USB; stereo audio via CPAL
- **Satellite tracking** — TLE‑based orbital prediction with pass list, elevation/azimuth plot, and auto‑tune to satellite downlink frequency at AOS
- **ADS‑B decoder** — real‑time aircraft tracking from Mode‑S replies (requires an RTL‑SDR or other wide‑band source)
- **Bookmarks** — named frequencies with category and mode tags; search/filter bar
- **Scheduler** — events with triggered actions (set frequency, toggle recording, switch mode, …)
- **Audio recording** — WAV capture of demodulated audio
- **AI assistant panel** — LLM integration for voice/text queries (configurable endpoint)
- **Web remote** — embedded HTTP server with a mobile‑friendly control page
- **MQTT** — publish frequency/status telemetry to an MQTT broker
- **Persistence** — settings, bookmarks, and scheduler events saved to JSON files (`ez_sdr_config.json`, `ez_sdr_bookmarks.json`)

## Build

### Dependencies

- **Rust** 1.75+ (edition 2021)
- **SoapySDR** development libraries (soapysdr, libsoapysdr-dev, or equivalent)
- **ALSA / PulseAudio / JACK** development headers (optional, for audio playback)

On Ubuntu/Debian:

```
sudo apt install build-essential libsoapysdr-dev
# Optional: audio support
sudo apt install libasound2-dev
```

On Fedora:

```
sudo dnf install gcc-c++ SoapySDR-devel
# Optional: audio support
sudo dnf install alsa-lib-devel
```

### Build & Run

```
cargo run --release
```

**Note:** The `audio` feature is enabled by default. To build without audio support (e.g., in containerized environments without ALSA):

```
cargo run --release --no-default-features
```

The first build compiles `dump1090` (the Rust ADS‑B decoder library) and `ez-gui` (the main application). Release builds are strongly recommended — debug builds are noticeably slower for spectrum rendering.

### Install from source

To install the binary system-wide via `cargo install`:

```
cargo install --path ez-gui --bin ez-gui
```

(Install `dump1090` the same way with `--bin dump1090`.)

## Quick Start

1. **Connect your SDR device** (RTL‑SDR, Airspy, HackRF, etc.)
2. **Run the application:** `cargo run --release` (or `ez-gui` if installed)
3. **Select your source** in the Source Manager panel (USB device, file, or network)
4. **Adjust frequency and gain**, pick a demodulation mode, and listen

For a faster first build (skip ADS‑B decoder), set `--no-default-features`. Re-enable with the `audio` feature flag or the `rtlsdr`/`soapy`/`hackrf` device backends as described in the `dump1090/` crate features.

## Testing

```
# Run the full test suite (both crates)
cargo test --workspace

# Run tests with all features
cargo test --workspace --all-features

# Lint check
cargo clippy --workspace --all-targets
```

[![CI](https://github.com/Lupc9102/ez-sdr/actions/workflows/ci.yml/badge.svg)](https://github.com/Lupc9102/ez-sdr/actions/workflows/ci.yml)

> **Security auditing:** Run `cargo audit` periodically to check for vulnerable dependencies (`cargo install cargo-audit` first).

## Development

Common commands are available via the `Makefile`:

| Command         | Action                        |
|-----------------|-------------------------------|
| `make check`    | `cargo check --workspace`     |
| `make test`     | `cargo test --workspace`      |
| `make clippy`   | `cargo clippy --workspace -- -D warnings` |
| `make fmt`      | `cargo fmt --check`           |
| `make fix`      | `cargo fmt`                   |
| `make clean`    | `cargo clean`                 |
| `make audit`    | `cargo audit`                 |

## Controls

| Control | Action |
|---|---|
| **Frequency** | Keyboard‑editable DragValue in the SDR panel (MHz) |
| **± step** | 10 kHz, 100 kHz, 1 MHz buttons |
| **Bandwidth / Sample rate** | Drop‑down of common SDR sample rates |
| **Gain** | 0–100 slider |
| **Mode** | RAW / AM / FM / WFM / LSB / USB |
| **FFT size / Window** | Drop‑downs above spectrum |
| **Averaging (α)** | Slider — 0.0 (instant) to 0.99 (heavily smoothed) |
| **Waterfall speed** | 1× / 2× / 4× / 8× |
| **Click‑to‑tune** | Left‑click anywhere on the spectrum plot |
| **Band plan** | Toggle (check box) — amateur bands from 160m to 70cm |

## Project Structure

```
ez-gui/      Main application (egui/eframe GUI)
  src/
    app.rs             Central application state and logic loop
    spectrum.rs        FFT, waterfall, spectrum plot
    source_manager.rs  SDR source configuration
    sdr_panel.rs       Left‑hand panel (frequency, gain, mode, …)
    satellite_panel.rs TLE engine + satellite list + pass table
    adsb_panel.rs      ADS‑B decoder UI
    adsb_decoder.rs    Wrapper around dump1090 decoder
    demod.rs           Demodulation modes
    audio_output.rs    Audio playback (CPAL)
    recorder_panel.rs  Audio recording to WAV
    scheduler.rs       Event scheduler
    bookmarks.rs       Frequency bookmarks with SQLite persistence
    database.rs        SQLite helper layer
    config.rs          Persistent settings
    web_remote.rs      Embedded HTTP server + WebSocket
    web_remote.html    Mobile web UI
    mqtt.rs            MQTT telemetry publisher
    tle_engine.rs      TLE download / orbital propagation
    ai_panel.rs        LLM assistant integration

dump1090/    Rust port of dump1090 Mode‑S/ADS‑B decoder (library)
  src/
    lib.rs             Public API
    demod.rs           Mode‑S demodulation (2400 baud)
    mode_s.rs          Mode‑S frame decoding
    mode_ac.rs         Mode‑A/C decoding
    cpr.rs             Compact Position Reporting
    track.rs           Aircraft track state
    net_io.rs          Network I/O (JSON output)
    sdr/               SDR device backends (RTLSDR, SoapySDR, file, …)
```

## Licence

MIT OR Apache-2.0, matching the `license` fields in each crate's `Cargo.toml`.

> **Owner action needed:** `dump1090/` is a port of GPL-2.0-licensed reference
> code (`dump1090`/`dump1090-fa` `mode_s.c`, `mode_ac.c`, `cpr.c`,
> `demod_2400.c`). A translation can be a derivative work, in which case the
> GPL would require the port to carry the GPL too — contradicting the MIT/Apache
> manifests. Previous revisions of this README claimed "GPL-2.0 or later — see
> the `COPYING` file", but no `COPYING` file exists. Resolve with counsel and
> then either relicense `dump1090` to GPL-2.0-or-later (adding `COPYING` +
> per-file attribution headers) or document why MIT/Apache stands.
