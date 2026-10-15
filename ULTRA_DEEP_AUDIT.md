# ez-sdr ultra-deep codebase audit

Date: 2026-09-11  
Scope: `/home/lupc/Documents/ez-sdr`  
Production code changed: **none**

## Grade

**Overall: C- (American grading system).**

The repository has a substantial test suite, clean default-feature checks, clear module boundaries, and several thoughtful defensive limits. That earns credit for engineering discipline. The grade drops below average because multiple advertised runtime paths are either uncompilable under documented features, wired to the wrong hardware ABI, silently lose or corrupt samples, or expose unauthenticated control and unsafe browser rendering. The default build is therefore a good development baseline, but the system is not production-ready across its stated SDR, GUI, LRPT, and web workflows.

For release readiness alone, the result is **D** until the P1 findings below are fixed and exercised with hardware/protocol fixtures.

## Evidence and validation

Passed checks:

- `cargo check --workspace --all-targets`
- `cargo test --workspace` (observed: dump1090 240 tests, ez-gui 496 unit + 4 integration, ez-proto 5, lrpt-decode 67)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all -- --check`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `cargo deny check advisories bans licenses sources`
- `npm test --prefix ez-web` (5 tests)
- `npm run build --prefix ez-web`
- Feature checks passed for ez-daemon `rtlsdr`, `hackrf`, and `soapy`, and dump1090 `hackrf` and `soapy`.

Failed or misleading checks:

- `cargo check --workspace --all-features` and `cargo test --workspace --all-features --no-fail-fast` fail because `dump1090/src/sdr/rtlsdr.rs:186-197` omits the `digital_agc` field while the constructor at `:214-225` and `start` at `:341-347` use it.
- `cargo check -p dump1090 --features rtlsdr` fails for the same reason. A documented RTL-SDR backend is not buildable.
- `cargo run --release` at workspace root is ambiguous among multiple binaries; only the package-qualified command works.
- Tests are predominantly synthetic/unit-level. They do not establish on-air LRPT image semantics, VCID interleaving, frame-gap recovery, malformed browser frames, or sustained hardware streaming.

## Highest-impact findings

### P1 — documented RTL-SDR feature is uncompilable

`dump1090/src/sdr/rtlsdr.rs:186-197` defines `RtlSdr` without `digital_agc`, but `:214-225` initializes it and `:341-347` reads it. This is a confirmed feature-gated build break hidden by default features. CI needs a matrix that compiles every backend combination.

### P1 — SoapySDR uses the wrong RX direction constant

`dump1090/src/sdr/soapy.rs:36` and `ez-daemon/src/hardware/soapy.rs:18` define `SOAPY_SDR_RX = 2`; upstream SoapySDR defines RX as `1` (TX is `0`). Every set-rate, frequency, gain, setup, and read call receives an invalid direction, so Soapy devices cannot work in practice. This needs an ABI smoke test or binding generated from the installed header.

### P1 — HackRF drops most callback data and corrupts the timebase

`ez-daemon/src/hardware/hackrf.rs:271-294` clears the accumulator, waits until `want_bytes`, converts only `buf.len()` complex samples, and discards any surplus bytes. HackRF callbacks are commonly much larger than the 16 KiB request used by ingestion, so most of a callback can be dropped on every read. The same accumulation pattern exists in `dump1090/src/sdr/hackrf.rs:313-337`. Surplus bytes and odd tails must be retained.

### P1 — live sample-rate changes leave active DSP configured for the old rate

`ez-daemon/src/state.rs:168-185` forwards the new rate to hardware but does not rebuild or retune channelizer FIRs, decimation, NCO assumptions, output metadata, or existing taps. An active channel continues processing at stale-rate assumptions while the source emits the new rate. This is reachable through REST, TCP, GUI daemon mode, and the web UI.

### P1 — default ADS-B rates are internally inconsistent

`ez-daemon/src/main.rs:61-62` defaults to 2.048 MHz, while `ez-daemon/src/pipelines/packet.rs:44-62` always constructs `Demod2400`, whose timing assumes 2.4 MHz. The web UI creates ADS-B channels at 2.0 MHz bandwidth (`ez-web/src/main.ts:150-154`), normally producing about 2.048 MHz. The default workflow is therefore theoretically mistimed and likely decodes poorly or not at all. The GUI has the same fixed-demod mismatch (`ez-gui/src/adsb_decoder.rs:56-70`).

### P1 — dump1090 overlap indexing duplicates old samples and skips new tail samples

`dump1090/src/main.rs:249-270` builds `[carry][new]`, and `dump1090/src/demod.rs:42-44` defines only `[overlap..valid_length]` as new. The scan at `dump1090/src/demod.rs:651-663` starts at index 0 and uses `valid_length - overlap`, so it rescans carry, excludes the newest overlap-sized tail, and shifts timestamps. This can duplicate messages and lose or delay boundary messages.

### P1 — GUI daemon mode is spectrum-only while presenting broader workflows

`ez-gui/src/source_manager.rs:367-388` subscribes only to `PipelineKind::Spectrum`. `ez-gui/src/app.rs:457-524` drains spectrum/error events and has no IQ path for daemon audio, ADS-B, local recording, or satellite recording. The UI can show Running or recording while those workflows receive no samples. Either implement matching subscriptions/consumers or disable unsupported controls.

### P1 — local GUI controls are snapshots, not live hardware controls

`ez-gui/src/source_manager.rs:169-180` captures frequency, rate, gain, AGC, and related settings when the worker starts. UI mutations at `ez-gui/src/app.rs:792-800`, `:864-881`, and `:1017-1019` do not reconfigure the worker. The displayed setting can change while the RTL-SDR/synthetic worker remains on the old value.

### P1 — GUI synthetic IQ uses absolute RF frequency and aliases the intended tones

`ez-gui/src/source_manager.rs:270-318` computes phase from `center_freq + offset` using the sample-index clock. At ordinary VHF/UHF centers this aliases unpredictably instead of generating baseband offsets, so the demo does not represent the signals its comments claim.

### P1 — GUI ADS-B demodulator ignores configured sample rate

`ez-gui/src/adsb_decoder.rs:56-70` ignores its `_sample_rate` argument and always uses `Demod2400`; the default GUI source is 2.048 MHz. Its timestamp fabrication (`:65-73`) assumes 5 clock ticks/sample although dump1090 uses a 12 MHz clock, so elapsed timing is also wrong.

### P1 — GUI audio clock and SSB oscillator are wrong

`ez-gui/src/demod.rs:235-242` uses integer decimation. 2.048 MHz to 48 kHz becomes 42, producing about 48,762 samples/s while CPAL is configured for 48 kHz (`ez-gui/src/audio_output.rs:54-62`). `ez-gui/src/demod.rs:749-768` advances the SSB oscillator once per input pair while dividing by the audio rate, shifting roughly 64 kHz instead of 1.5 kHz at 2.048 MHz.

### P1 — LRPT reassembly ignores VCID and frame continuity

`lrpt-decode/src/lib.rs:304-316` parses and discards the VCDU header, including VCID and frame counter. `lrpt-decode/src/ccsds.rs:192-225,287-301` keeps one global active APID continuation context. Interleaved virtual channels can append continuation bytes to the wrong packet, and frame loss does not reset stale partial packets. No fixture covers VCID interleave or counter gaps.

### P1 — LRPT image extraction is an unverified assumption

`lrpt-decode/src/lib.rs:318-321` sends each APID 64–69 packet body directly to `ImageBuilder::push_scanline`. `lrpt-decode/src/image_builder.rs:15-18,75-82` documents the missing packet-specific sub-header handling as an assumption. There is no line-number/sequence validation, sub-header stripping, or golden on-air Meteor image. Passing synthetic round trips do not prove production imagery.

### P1 — LRPT progress/rendering scales quadratically under live decode

`lrpt-decode/src/lib.rs:212-219,243-244,325-340` throttles progress by input calls rather than decoded CADUs and calls `render_all()` repeatedly. `image_builder.rs:151-169` clones all accumulated pixels; `ez-daemon/src/pipelines/telemetry.rs:85-98` clones again and republishes full images. A growing image therefore causes O(N²) pixel traffic and allocations, with queue/heap pressure from multiple snapshots.

### P1 — LRPT web channel is exposed but not consumable

`ez-web/src/ui/channel-list.ts:7` offers `LrptTelemetry`, and types/wire decoders exist, but `ez-web/src/main.ts:66-104,150-156` only attaches Spectrum and ADS-B streams. There is no telemetry renderer or stream consumer, so a browser user can create a channel that has no visible result.

### P1 — unauthenticated remote control and DOM XSS

The daemon exposes frequency, gain, sample rate, channel, recording, and demod controls over TCP/HTTP/WebSocket. `ez-daemon/src/main.rs:43-54` permits arbitrary listen addresses. The WebSocket control path explicitly omits the TCP Hello/version handshake (`ez-daemon/src/web/ws.rs:130-145`). In the frontend, `ez-web/src/ui/aircraft-panel.ts:70-85` interpolates radio-derived `callsign` into `innerHTML` without escaping. Binding beyond loopback can therefore expose control and data, and a hostile callsign can execute script in the daemon origin.

## Additional important findings

- Soapy `readStream` negative statuses are treated as fatal at `ez-daemon/src/hardware/soapy.rs:382-394`; upstream uses negative codes for timeout/overflow conditions that should be handled distinctly. A normal USB/driver timeout can kill ingestion.
- `ez-daemon/src/state.rs:140-165` validates `abs(offset) <= fs/2` but not `abs(offset) + bandwidth/2 <= fs/2`; channel edges can alias outside capture. `i64::MIN.abs()` can also panic.
- Hardware setters update cached fields before FFI success (`ez-daemon/src/hardware/rtlsdr.rs:241-245`, HackRF analogous), while `ez-daemon/src/ingest.rs:123-139` can overwrite an error snapshot with success after processing another command. API/UI state can lie about hardware state.
- Channel deletion (`ez-daemon/src/state.rs:470-490`) stops a flag and drops the pipeline, but existing forwarder threads and recordings are not explicitly shut down. Repeated create/delete/subscription activity can accumulate blocked resources.
- TCP handshake waits indefinitely at `ez-daemon/src/server.rs:126-157`; there is no handshake timeout or connection cap. Per-subscription OS threads amplify unauthenticated resource exhaustion.
- `ez-daemon/src/ingest.rs:63-74` evicts an arbitrary oldest command from a shared queue, so a frequency flood can discard unrelated gain or sample-rate commands.
- `ez-daemon/src/app.rs:112-119` uses `tokio::join!`; if one server task fails, the sibling can continue and `run()` may wait indefinitely despite comments implying coordinated teardown.
- `ez-daemon/src/web/api.rs:174-187` always returns `201` and echoes the request even when `subscribe` reuses an existing channel ID with different configuration.
- `ez-web/src/api/wire.ts:9-54` trusts lengths and counts without bounds/consistency checks. A malformed frame can throw `RangeError`, terminate the spectrum worker, or silently truncate audio/telemetry. `aircraft-panel.ts:59-85` similarly trusts arbitrary JSON shapes.
- `ez-web/src/audio/monitor.ts:26-40,74-84` can resurrect an audio node after `stop()` because in-flight async initialization has no generation token.
- `ez-gui/src/source_manager.rs:197-229` silently leaves replay status Running after EOF, and the GUI accepts `.cf32` at `:552-554` even though `ez-gui/src/satellite/recorder.rs:163-171` writes little-endian float pairs. Replay sends those float bytes as unsigned 8-bit IQ.
- Public LRPT constructors accept invalid rates (`lrpt-decode/src/lib.rs:143-165`); zero/NaN symbol-rate paths can produce NaN state or an infinite timing loop in `qpsk.rs:217-224,313-363`. Arbitrary NaN/Inf CF32 input is also not sanitized (`iq_source.rs:35-43`).
- FrameSync accepts Hamming distance four, making false locks plausible in noise, and truncates large input chunks before searching (`lrpt-decode/src/frame_sync.rs:42-45,149-157`; `decode_file` feeds MiB chunks). There are no large-chunk or false-lock regressions.
- Satellite tuning and recording also have correctness gaps: `ez-gui/src/tle_engine.rs:366-375` falls back to 100 MHz for Meteor-M2-3/M2-4; `ez-gui/src/satellite_panel.rs:97-140` continuously overwrites manual tuning; `satellite/recorder.rs:58-134` reports success before file creation and drops full queues silently; CF32 metadata is hardcoded to 2.048 Msps (`satellite_panel.rs:401-415`); WAV metadata is hardcoded to 48 kHz (`recorder_panel.rs:317-325`) despite device-dependent rates; RF decimation and anti-aliasing are incomplete (`demod.rs:350-352,431-441,568-586`).

## What is good

- Default-feature Rust and frontend gates are reproducible and clean.
- Unit coverage is broad across DSP primitives, protocol codecs, and GUI components.
- The daemon has explicit channel caps, finite-rate validation in several control paths, bounded broadcaster queues, and clear status snapshots.
- Code comments often document intended invariants and shutdown behavior, making the defects tractable to fix.

## Recommended repair order

1. Make every documented backend build and operate: fix `digital_agc`, Soapy RX constant, Soapy status codes, HackRF buffering, and add a hardware/ABI CI matrix.
2. Establish one sample-rate contract. Rebuild channelizer/DSP state on rate changes and use a real fractional resampler or constrain supported rates. Align ADS-B demod/timestamps with actual rate.
3. Fix GUI daemon capabilities and local reconfiguration; correct baseband synthetic generation, `.cf32` conversion, EOF state, and audio/SSB clocks.
4. Make LRPT protocol handling production-grade: preserve VCID/frame counters, reset on gaps, parse image subheaders/line sequence, sanitize rates/floats, and publish deltas or bounded previews instead of full-image clones.
5. Add authentication/authorization or hard loopback-only defaults, origin/version checks, handshake timeouts, connection limits, and explicit shutdown of forwarders/recordings.
6. Harden the web boundary with length/schema validation, safe DOM construction, async lifecycle cancellation, hostile-payload tests, and an actual LRPT renderer.

## Bottom-line assessment

The codebase is coherent enough to develop and test, but its green default checks overstate readiness. The failures are concentrated at integration boundaries: feature flags, C ABIs, rate/timestamp contracts, stream buffering, protocol continuity, and UI/security trust boundaries. Those are exactly the boundaries that determine whether an SDR application works with real hardware and real traffic. Until the P1 list is addressed and covered by end-to-end fixtures, treat ez-sdr as an actively developing prototype rather than a deployable receiver stack.
