# ez-sdr

A native Rust SDR desktop with three workspaces: **Radio**, **ADS-B**, and **Meteor**.
The UI uses egui with a single OpenGL renderer, compact controls, a dark theme, and
background file decoding. It covers the requested SDR++-style radio workflows while
keeping the layout lightweight; exact SDR++ plugin ordering is not required for use.

## Run

```bash
cargo run -p ez-gui --release

# Live RTL-SDR reception (requires librtlsdr):
cargo run -p ez-gui --release --features rtlsdr
```

The app opens with the receiver stopped. Select **Demo**, **RTL-SDR**, **File replay**,
or **Daemon** in the Source section, then press Play. Demo produces generated signals;
it does not receive local stations. Without the `rtlsdr` feature, use a hardware-enabled
`ez-daemon` or a recording to receive real signals.

On Debian/Ubuntu, the default audio build needs `build-essential libasound2-dev`.
Add `librtlsdr-dev` for the RTL-SDR feature. Rust 1.92 or newer is required. To build
without speaker playback, use `--no-default-features`.

## Radio

- Play/stop, per-digit frequency tuning, direct frequency entry, mute and volume are
  in the top receiver bar. Scroll a digit or click its upper/lower half; double-click
  the readout to type a frequency with a unit such as `118.1 MHz`.
- The sidebar exposes source selection, sample rate, gain, hardware options, modulation,
  bandwidth, squelch, audio processing, and spectrum/waterfall display controls.
- RTL-SDR device discovery runs in the background. Use **Refresh** after plugging in a
  receiver; a unique USB serial keeps the selected receiver stable across index changes.
  **Offset Tuning** is available when the tuner supports it.
- Use **AM** for aviation voice (typically 118–137 MHz), **WFM** for broadcast FM,
  **NFM** for narrow FM voice, and **USB/LSB** for sideband signals. Choose a local,
  published frequency and an antenna suitable for that band.
- **DSB** uses product detection; **CW** adds a configurable beat tone after its narrow
  RF channel filter. **Bandwidth** controls RF selection independently of **Audio cutoff**.
  WFM **Stereo** decodes the pilot/multiplex and falls back to mono without a usable pilot;
  optional **RDS** decoding exposes station text and metadata.
- Choose the output device and sample rate under **Audio**. An unavailable saved device
  produces an error; it is not silently replaced by another device.
- Display controls support streaming FFT sizes through 65,536, FFT rate, waterfall
  visibility and SNR smoothing. The waterfall and drawn traces remain bounded in size.
- Click the spectrum to tune. Frequency changes retune an active local source; changing
  a frequency while stopped does not start reception.
- The collapsed **SDR++ Modules** section exposes Recorder, Sinks, Frequency Manager,
  Band Plan, and the loopback **Rigctl Server**. Rigctl accepts Hamlib-style `f/F`,
  `m/M`, `v/V`, and `q` commands on localhost (default port `4532`).
- General recording, bookmarks, settings and other utilities are under **Tools**.

DSB/CW and stereo decoding apply to local RTL-SDR, replay and Demo sources; the current
daemon audio protocol lacks DSB/CW and supplies mono audio. Native reception and audible
output still require hardware validation, and exact SDR++ plugin spacing is an optional
follow-up; see `UI_FINAL_SESSION_SUMMARY.md`.

## ADS-B

The map shows heading-rotated SVG aircraft, trails, an aircraft list and OpenStreetMap
attribution. Set your observer position in Settings to center reception coverage. Map
image tiles are downloaded as needed and cached locally; uncached areas need internet.

### 1090 MHz — Mode S / 1090ES

Select a working SDR source, then open **ADS-B** and choose **1090 MHz**. The local
receiver uses 2.4 MSPS. A daemon source can deliver decoded aircraft instead. The Demo
source does not simulate real local aircraft. Reception requires a suitable 1090 MHz
antenna and line of sight.

### 978 MHz — UAT (United States)

978 MHz receives decoded reports from **dump978-fa** over its direct JSON TCP feed.
The external decoder owns the SDR; selecting 978 stops EZ-SDR's local source so both
programs do not compete for one device.

Install/build [dump978-fa](https://github.com/flightaware/dump978) with SoapySDR and the
appropriate device driver, then run it separately:

```bash
dump978-fa --sdr driver=rtlsdr --json-port 127.0.0.1:30979
```

Choose **978 MHz** in ADS-B, use `127.0.0.1:30979` (or another numeric IP and port), and
connect. Connection errors are displayed and the worker retries without blocking the UI.
Anonymous/non-ICAO addresses remain separate from ICAO aircraft. The Rust app does not
itself demodulate UAT I/Q. The external decoder is a runtime dependency for this band.

## Meteor — imported recordings only

1. Open **Meteor** and browse for an existing `.cs8` or `.cf32` recording.
2. Set the recording's actual sample rate and symbol rate. CS8 means signed interleaved
   8-bit I/Q; CF32 means little-endian float32 I/Q. These formats are not interchangeable.
3. Choose a Meteor preset or Custom, then Decode. Progress and reconstructed channels
   appear while a background worker reads the file in bounded blocks.
4. Cancel a running decode or export the decoded channels as PNGs. Repeated exports
   get unique filenames rather than overwriting an earlier pass.

Meteor has independent decode state and no source/recording controls. No SDR, observer
location or TLE is needed to decode an existing recording. The decoder targets
Meteor-M2-3/M2-4 LRPT OQPSK at 80 or 72 ksym/s. NOAA APT, HRPT and GOES are not implemented.
Synthetic end-to-end and protocol tests are available; independent off-air recordings
are still needed for field validation.

## Optional daemon

The Rust daemon owns source/DSP pipelines and exposes the desktop's binary protocol
at `127.0.0.1:7890`, plus HTTP/WebSocket APIs on `127.0.0.1:7891`.
The previous `ez-web` frontend is not included in this worktree.

```bash
cargo run -p ez-daemon --bin ez-daemon --release
cargo run -p ez-daemon --bin ez-daemon --release --features rtlsdr -- --source rtlsdr
cargo run -p ez-daemon --bin ez-daemon --release --features soapy -- \
  --source soapy --device 'driver=airspy'
```

Choose **Daemon** in the desktop Source section and connect to its binary endpoint.
HackRF and SoapySDR backends require their corresponding feature and system libraries.
Use `cargo run -p ez-daemon --bin ez-daemon -- --help` for source and bind options.

## Development and verification

```bash
cargo test --workspace --no-default-features
cargo check -p ez-gui --features rtlsdr

# Render actual egui widgets without a window server:
EZ_SDR_PREVIEW_DIR=/tmp/ez-sdr-previews cargo test -p ez-gui --no-default-features \
  app_ui_tests::render_workspaces -- --ignored --nocapture
```

Network integration tests need permission to bind loopback sockets. The CPU preview
renderer checks egui layout and textures; native window/input/audio validation remains
separate. No physical SDR is covered by the automated tests.

Current task/evidence: [UI_FINAL_SESSION_SUMMARY.md](UI_FINAL_SESSION_SUMMARY.md) and
[UI_BUILD_TASK.md](UI_BUILD_TASK.md).
Earlier reports are historical and may describe interfaces that have since changed.

## License

Workspace manifests declare `MIT OR Apache-2.0`. The existing `dump1090` Rust port has an
unresolved licensing review against its GPL reference implementations; resolve attribution
and licensing before distribution. `dump978-fa` remains a separately installed program.
