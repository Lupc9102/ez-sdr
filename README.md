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
- **Persistence** — all settings, window geometry, bookmarks, and scheduler events saved to an SQLite database

## Build

### Dependencies

- **Rust** 1.75+ (edition 2021)
- **SoapySDR** development libraries (soapysdr, libsoapysdr-dev, or equivalent)
- **ALSA / PulseAudio / JACK** development headers (for CPAL audio backend)

On Ubuntu/Debian:

```
sudo apt install build-essential libsoapysdr-dev libasound2-dev
```

On Fedora:

```
sudo dnf install gcc-c++ SoapySDR-devel alsa-lib-devel
```

### Build & Run

```
cargo run --release
```

The first build compiles `dump1090` (the Rust ADS‑B decoder library) and `ez-gui` (the main application). Release builds are strongly recommended — debug builds are noticeably slower for spectrum rendering.

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

GPL‑2.0 or later — see the `COPYING` file.
