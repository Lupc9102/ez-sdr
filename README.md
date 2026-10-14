# ez-sdr

A native Rust SDR desktop with three workspaces: **Radio**, **ADS-B**, and **Meteor**.
The UI uses egui with a single OpenGL renderer, compact controls, a dark theme, and
background file decoding. It implements local radio listening, native 1090 MHz
ADS-B reception and offline Meteor import. It has narrower hardware, plugin and
product support than SDR++ and SatDump.

## Run

```bash
cargo run -p ez-gui --release

# Audio playback without the RTL-SDR build dependency:
cargo run -p ez-gui --release --no-default-features --features audio
```

The app opens with the receiver stopped. Select **Demo**, **RTL-SDR**, **File replay**,
or **Daemon** in the Source section, then press Play. Demo produces generated signals;
it does not receive local stations. Without the `rtlsdr` feature, use a hardware-enabled
`ez-daemon` or a recording to receive real signals.

The default build includes audio and RTL-SDR. On Debian/Ubuntu, install
`build-essential pkg-config libasound2-dev libpulse-dev librtlsdr-dev`.
The repository pins Rust 1.92. To build without speaker playback and RTL-SDR,
use `--no-default-features`; to retain only RTL-SDR, add `--features rtlsdr`.

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
- FM audio uses physical deviation: unit amplitude represents 5 kHz in NFM and
  75 kHz in WFM, before volume and audio processing. The deviation readout measures
  half the received frequency span over completed 100 ms windows, ignoring carrier
  offset and the first 10 ms of filter acquisition. Stereo and mono share this scale.
- Native signal measurements estimate the strongest carrier peak in the selected
  RF channel, in dBFS; they do not integrate total RF power. USB/LSB use their
  selected sideband. Narrow CW measurements need a complete window (up to 200 ms);
  retunes and capture gaps clear the previous reading. Display FFT settings are independent.
- Replay scans validate the requested scan grid and bookmark passbands against the
  file's fixed coverage. An unavailable channel stops the scan with its frequency
  and available tuning range; valid scans still require fresh observations after tuning.
- Choose the output device and sample rate under **Audio**. An unavailable saved device
  produces an error; it is not silently replaced by another device.
- Display controls support streaming FFT sizes through 65,536, FFT rate, waterfall
  visibility and SNR smoothing. The waterfall and drawn traces remain bounded in size.
- Click the spectrum to tune. Frequency changes retune an active local source; changing
  a frequency while stopped does not start reception.
- **Setup Guide** and **Settings** open as separate floating windows from **Tools**.
- Bookmarks remain available from the spectrum. Tune with direct frequency entry
  or by selecting a frequency marker. The
  loopback **Rigctl Server** provides a small Hamlib command subset on localhost
  (default port `4532`): frequency `f/F`; the receiver's supported `m/M` modes and
  passband; `v` returns `VFOA`, while `V` accepts only `VFOA`/`currVFO`; and `l AF`/
  `L AF` read or set normalized audio gain from `0.0` to `1.0`. It does not implement
  full rigctld or a second VFO; `q` closes the client connection.

### Settings and retained integrations

Open **Tools → Settings** to configure MQTT publishing, the loopback Web Remote,
and the loopback rigctl server. The Web Remote is available at
`http://localhost:5259` by default and carries the browser's recording and
scanner controls (`start_scan` / `stop_scan`). The **Satellite** workspace has
the pass auto-tune switch; the scheduler engine continues to apply pass windows
and cooldown rules. The reusable `SchedulerPanel` API can expose custom tasks in
an embedding app, but the desktop does not mount it as a utility window.

Discord notifications run in the background and report delivery results in the
status strip. Configure them by importing a settings JSON file from **Settings**
with a `discord` object, for example `{"discord":{"enabled":true,"bot_token":"…","channel_id":"…","ping_user":false}}`.
The default settings export redacts credentials; opt into a private backup when
you need to move credentials between installations. The standalone AI and
Discord panel modules remain available to downstream embedders, but the desktop
workspaces do not mount those utility panels. Legacy `ai_*` and custom-theme
fields remain readable in settings files for compatibility.

The desktop supports two simultaneous listening receivers sharing the same capture.
Enable **Independent B receiver** in the Radio sidebar; its frequency must fit
inside the receiver capture, and its mode, squelch and volume are independent.
V swaps A/B frequencies. Daemon mode uses a separate authenticated B channel.

DSB/CW are supported locally and over the daemon protocol. WFM stereo uses pilot
recovery on both paths and falls back to dual-channel mono when no pilot locks.
Native reception and audible output still require hardware validation.

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

The **Native IQ** source is selected by default. EZ-SDR decodes UAT downlink and
uplink frames in Rust from the shared capture stream; no separate dump978 process is
needed. Native decoding requires a 978 MHz-centered capture at exactly 2,083,333
complex samples/second. The receiver reports a clock or tuning mismatch instead of
silently decoding an unsupported capture.

For compatibility with an existing receiver, choose **dump978-fa JSON** to use its
direct JSON TCP feed. Install/build [dump978-fa](https://github.com/flightaware/dump978)
with SoapySDR and the appropriate device driver, then run it separately:

```bash
dump978-fa --sdr driver=rtlsdr --json-port 127.0.0.1:30979
```

Choose **978 MHz** in ADS-B, enter `127.0.0.1:30979` (or another numeric IP and port),
and connect. Connection errors are displayed and the worker retries without blocking the
UI. External JSON mode releases EZ-SDR's SDR so the external receiver can own it.
Anonymous/non-ICAO addresses remain separate from ICAO aircraft in either source mode.

## Meteor — live passes and imported recordings

1. Open **Meteor** and browse for an existing `.cs8` or `.cf32` recording.
2. Set the recording's actual sample rate and symbol rate. CS8 means signed interleaved
   8-bit I/Q; CF32 means little-endian float32 I/Q. These formats are not interchangeable.
3. Choose a Meteor preset or Custom, then Decode. Progress and reconstructed channels
   appear while a background worker reads the file in bounded blocks.
4. Cancel a running decode or export the decoded channels as PNGs. Repeated exports
   get unique filenames rather than overwriting an earlier pass.

Meteor has independent live and imported-file decode state. **Start live** starts
the selected receiver; **Stop live** finalizes decoded channels for export.
No SDR, observer location or TLE is needed to decode an existing recording. The decoder targets
Meteor-M2-3/M2-4 LRPT OQPSK at 80 or 72 ksym/s. NOAA APT, HRPT and GOES are not implemented.
Each reconstructed channel is limited to 16,384 image lines. Live capture and
RGB/crop/rotation/histogram products are supported. Geolocation uses absolute
packet timestamps, the selected satellite TLE and explicit scan geometry.
Public off-air 72k and independently generated 80k recordings have packet/image
checks; an original off-air 80k pass and target receiver validation remain open.

New CF32 recordings contain little-endian float32 I/Q in normalized full-scale units
(CU8 values 0 and 255 map to -1 and +1). Their JSON sidecar declares
`"amplitude": "normalized"`. Replay and Meteor imports read this declaration; the
known older satellite-recorder sidecar selects its legacy `byte - 127.4` units.
Other files without amplitude metadata default to normalized units. For an older
unannotated recording, select **Legacy ADC counts** in Replay, pass the daemon
`--replay-amplitude adc-counts-127-4`, or add `"amplitude": "adc_counts_127_4"`
to its JSON sidecar before a Meteor import. Units are never guessed from signal peaks.

Local **RAW** listening is stereo I/Q audio: I on the left and Q on the right,
filtered to the audio Nyquist limit and resampled to the selected sound-device
clock. WAV recordings retain both channels and the capture's real duration,
including fractional RF clocks and incomplete RF decimation strides. RAW uses
receiver IQ correction and RF decimation, volume and audio gain (0.3 default
headroom), and bypasses the mono voice EQ/AGC/pitch chain. Use **IQ recording** for
full-band samples at the original RF clock.

AM, SSB, DSB and CW listening filter the detected audio before converting it to
the selected sound-device rate. This also removes harmonics created by AM
envelope detection and frequencies shifted above audio Nyquist by the CW beat.
Audio filters and effects run at the device clock; RF bandwidth remains separate
from audio bandwidth.

Daemon USB/LSB listening preserves voice pitch and rejects the opposite sideband
across the 300–3,000 Hz voice band, including the GUI’s 200 kS/s channel.
Daemon AM/FM/WFM select complex RF before detection and measure power squelch on
that selected signal. Their default RF widths are 15/25/180 kHz. The desktop
requests 200 kS/s channels for voice modes and 256 kS/s for WFM, with 48 kS/s
PCM delivery (two interleaved channels when WFM stereo is selected); integer RF decimation can produce a higher actual channel rate.

Automatic recording and signal monitoring require **Power** squelch and use the
entire -100..0 dBFS threshold range. Local observations come from the native RF
processor; daemon observations use fresh raw spectrum measurements after capture
settings and the power threshold are acknowledged. Monitoring continues with the
Listen tab hidden and the display frozen. A missing or expired measurement waits
for fresh input instead of opening the trigger. Automatic recording counts advance
only after the local file opens or the daemon confirms an active recording.

## Optional daemon

The GUI and daemon must both speak protocol version 13; rebuild both together. Aircraft telemetry preserves address class, observation timestamps, emergency state, position decode quality and signed optional measurements. Ground speed/track, IAS/TAS and magnetic/true heading are separate. Aircraft snapshots include their channel identity, receiver counters and a fixed five-second message rate that expires during idle reception. Welcome allocates connection-owned GUI channel IDs; all streamed data carries channel identity and control subscribers receive explicit removal events. Active recordings prevent channel deletion. Daemon squelch commands have correlated applied/error replies, and audio frames carry the applied threshold and revision so queued audio cannot undo a newer setting.

Audio subscriptions separate `channel_sample_rate_hz` from
`audio.rf_bandwidth_hz` and `audio.output_sample_rate_hz`. JSON requests accept
the former `bandwidth_hz` name as an alias for the channel rate. Omitting `audio`
uses mode defaults and follows the actual channel clock; an explicit output rate
of zero also selects that behavior. `Retune.audio` applies mode, RF width and PCM
rate together with channel geometry. Audio frames retain physical RF provenance
in `clock` and carry the generated PCM timeline separately in `audio_clock`.

LRPT subscriptions require `meteor_profile: "Oqpsk72"` or
`meteor_profile: "Oqpsk80Interleaved"`. These select the complete transport,
including NRZ-M and the 80k marker/deinterleaver stage. Other pipeline kinds
leave this field absent. The actual channel output rate must provide at least
four samples per symbol (288 kS/s or 320 kS/s), and the requested RF window must
contain the occupied 108 kHz or 120 kHz band. Unsupported requests and retunes
fail before the live channel changes.

Capture gaps discard partial decoding but preserve the current image product.
A source session change, replay loop or frequency retune starts a fresh product.
To begin another pass at the same tuning, send
`{"StartTelemetryProduct":{"channel_id":7}}` over either control transport.
Telemetry previews carry `product_id`; a zero-sized APID-zero preview announces
a product boundary, and clients must retire images from its prior ID. The raw
telemetry WebSocket header is now 36 bytes: the product ID is a little-endian
u64 at offset 28, followed by pixels at offset 36. A timestamp trailer follows
those pixels: a little-endian u32 row count and that many f64 Unix timestamps
(NaN means unavailable). Flags bits 2 and 3 mark a full final channel and pass
completion. **Stop live** finalizes a daemon pass before unsubscribing; the
browser provides **Finalize pass**. Disconnecting the receiver first aborts
finalization and reports incomplete products. Final images retain full resolution
and packet row times for the same product/export tools used by local capture.

For initial surface ADS-B positions, configure the daemon with `--receiver-lat 51.5 --receiver-lon -0.1` using the actual IQ capture site. Without a configured receiver or a fresh aircraft reference, ambiguous surface positions stay unavailable. For replay, the reference must describe the recording location.

The Rust daemon owns source/DSP pipelines and exposes the desktop's binary protocol
at `127.0.0.1:7890`, plus HTTP/WebSocket APIs on `127.0.0.1:7891`.
The daemon serves its bundled browser console at `/` when no external static directory is
configured. A supplied `--web-static-dir` can replace those bundled assets.

```bash
cargo run -p ez-daemon --bin ez-daemon --release
cargo run -p ez-daemon --bin ez-daemon --release --features rtlsdr -- --source rtlsdr
cargo run -p ez-daemon --bin ez-daemon --release --features soapy -- \
  --source soapy --device 'driver=airspy'
```

Choose **Daemon** in the desktop Source section and connect to its binary endpoint.
HackRF and SoapySDR backends require their corresponding feature and system libraries.
Use `cargo run -p ez-daemon --bin ez-daemon -- --help` for source and bind options.
The daemon defaults to loopback. Authentication is optional for local development; enable a
shared token for clients with `--auth-token` or `EZ_DAEMON_AUTH_TOKEN`:

```bash
EZ_DAEMON_AUTH_TOKEN='replace-with-a-random-secret' \
  cargo run -p ez-daemon --bin ez-daemon --release -- --bind 0.0.0.0
```

REST clients send `Authorization: Bearer <token>`. Browser WebSockets offer the
`ez-sdr-auth.<lowercase-hex-utf8-token>` subprotocol because browser WebSocket APIs cannot
set an Authorization header. Binary TCP clients send `Authenticate` before `Hello`.
Without a configured token, old local TCP clients and unauthenticated REST/WebSocket clients
continue to work.

HTTP and WebSocket requests require a localhost or numeric-IP Host. Browser requests
also require a trusted loopback Origin or the same numeric IP/port endpoint; native
clients may omit Origin. These checks cover browser request boundaries and do not
authenticate clients. A token sent over plain HTTP/TCP is visible to network observers, so
for remote use place the daemon behind a TLS-enabled authenticated proxy with an appropriate
origin policy and a localhost/numeric upstream Host. Keep a directly exposed daemon on a
trusted network.

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
Use [HARDWARE_VALIDATION.md](HARDWARE_VALIDATION.md) to record the receiver,
audible-output and original 80k Meteor checks before making field-reliability claims.

Current task/evidence: [UI_FINAL_SESSION_SUMMARY.md](UI_FINAL_SESSION_SUMMARY.md) and
[UI_BUILD_TASK.md](UI_BUILD_TASK.md).
Earlier reports are historical and may describe interfaces that have since changed.

Dependency maintenance: `bincode` 1.x remains unmaintained. The documented
`RUSTSEC-2025-0141` exception in `deny.toml` depends on bounded codec frames and
the enforced protocol handshake. Replacing it requires an explicit wire-version
migration across the daemon and clients; the exception does not waive new advisories.

## License

The translated `dump1090` crate declares `GPL-2.0-or-later AND BSD-3-Clause`;
independently authored workspace components retain `MIT OR Apache-2.0`.
Combined release binaries are distributed under GPL-3.0-or-later with upstream
attribution, license texts and matching vendored source. See [LICENSE.md](LICENSE.md)
and [DISTRIBUTION.md](DISTRIBUTION.md) for the scope and Linux package instructions.
`dump978-fa` is optional and only needed for its JSON source mode.
