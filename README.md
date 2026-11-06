# ez-sdr

One of *THE FIRST* Rust SDR app, it has three workspaces: 
**Radio** (Pretty self explainatory)
**ADS-B** native 1090 and 978 MHz ADS-B reception
**Meteor** LRPT image decoding
built with egui and a single OpenGL renderer.

## Install

Linux x86-64 installers are in [Releases](https://github.com/Lupc9102/ez-sdr/releases/latest).
On Ubuntu based distros:

```bash
sudo apt install ./ez-sdr_*_amd64.deb
```

Or make the AppImage executable and open it. Checksums and supported systems are
in [DISTRIBUTION.md](DISTRIBUTION.md).

Build from source (Debian/Ubuntu: `build-essential pkg-config libasound2-dev
libpulse-dev librtlsdr-dev`):

```bash
cargo run -p ez-gui --release
cargo run -p ez-gui --release --no-default-features --features audio
```

The app opens with the receiver stopped. Pick **Demo**, **RTL-SDR** (Should be auto detected if you have a Dongle plugged in), **File replay**,
or **Daemon** in the Source section, then press Play.
Intuitively, **Demo generates signals it
doesnt receive real stations**

## Radio

- **Tuning**: scroll any digit of the frequency readout, double click to type a
  frequency (format is `XXX.XXXXXX MHz`), or click the spectrum to jump to the specific freq. The readout should keep
  1 Hz precision.
- **Modes**:
- **AM** for aviation (Airport ATC)
- **WFM** for broadcast FM
- **NFM** for narrow FM
- **USB/LSB** for sideband
- **DSB**.

- **The Sidebar has**:
- source selection
- sample rate
- gain
- modulation
- bandwidth
- squelch and audio processing
- FFT/waterfall settings
- Advanced hardware filter
- display options (hiden under collapsible sections)
- bandwidth and tune-step fields ONLY accept `Hz`/`kHz`/`MHz` units.
- **RTL-SDR Support**: Should be auto detected while opening the app, if it fails then press refresh, If you plugged in the dongle AFTER opening the app, also press refresh.
- **Recording**: WAV voice recordings, stereo I/Q **RAW** listening, full-band **IQ recording** at the original RF clock, and automatic recording triggered off the power squelch.
  
## ADS-B

The map shows heading rotated aircraft, trails, an aircraft list and OpenStreetMap tiles.
While not necessary you can also set your own coordinates in settings.

- **1090 MHz**: Most planes worldwide use this.
- **978 MHz**: Freedom frequency, and apparently some European drones use this too.

## Meteor

Live and offline decoding of Meteor-M2-3/M2-4 **LRPT** (OQPSK, 80 or 72 ksym/s).
!! NOAA APT, HRPT and GOES are not implemented. (Because the only 2 LRPT Meteorological Sats still broadcasting are both Meteor sats)

1. **Meteor → Open recording**, pick a `.cs8`  or
   `.cf32` (TIP: you can use FFMPEG to change your recording format)
2. Set the recording's actual sample rate and symbol rate, choose a Meteor preset
   or Custom, then **Decode**. Thats it!!
   Over time blocks, progress and reconstructed channels appear live.
3. You can also export your decoded images as .png's.


## Optional daemon

The Rust daemon owns the source/DSP pipelines headless, exposing the desktops
binary protocol at `127.0.0.1:7890` and HTTP APIs at `127.0.0.1:7891`
GUI and daemon must speak the same protocol version. I recommend rebuilding them together.

```bash
cargo run -p ez-daemon --bin ez-daemon --release
cargo run -p ez-daemon --bin ez-daemon --release --features rtlsdr -- --source rtlsdr
cargo run -p ez-daemon --bin ez-daemon --release --features soapy -- \
  --source soapy --device 'driver=airspy'
```

Then pick **Daemon** as the desktop source and connect.

## Development

```bash
cargo test --workspace --no-default-features
cargo check -p ez-gui --features rtlsdr

# render egui widgets without a window server:
EZ_SDR_PREVIEW_DIR=/tmp/ez-sdr-previews cargo test -p ez-gui --no-default-features \
  app_ui_tests::render_workspaces -- --ignored --nocapture
```

The CPU preview renderer checks layout and textures, physical SDR, audible output and original
80k Meteor checks are recorded in HARDWARE_VALIDATION.md.
bincode 1.x is unmaintained the RUSTSEC-2025-0141 exception in deny.toml
depends on bounded codec frames and the enforced protocol handshake.

## License

The translated `dump1090` crate declares `GPL-2.0-or-later AND BSD-3-Clause`
independently authored workspace components retain `MIT OR Apache-2.0`.
Combined release binaries are distributed under GPL-3.0-or-later with upstream
attribution, license texts and matching vendored source. See [LICENSE.md](LICENSE.md)
and [DISTRIBUTION.md](DISTRIBUTION.md) for the scope and Linux package instructions.
`dump978-fa` is optional and only needed for its JSON source mode.


## AI tools used in this project

Kimi K2.6 was used to lay the foundation, Sonnet 5 handled most of the implementations and bug fixes. Fable 5 and Opus 4.8 were used to refine the LRPT decoding pipeline. Several Chinese AI models were used for quick changes and patches, while GPT-5.6 Sol, GPT-5.6 Terra, GPT-5.6 Luna, GPT-6 Luna, GPT-6 Sol, GPT-6 Astra, and GPT-6.1 Sol were **all** used to hunt major bugs and get the project ready to ship.

AI played a *huge* role in writing and improving the code (mostly because I only started learning Rust this June), but the project's direction, design decisions, physical and digital testing, and overall development were guided by me.
