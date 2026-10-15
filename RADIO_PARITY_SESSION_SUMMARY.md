# Radio parity continuation — 2026-09-23

## Outcome

Implemented the next working receiver features and built the native RTL-SDR release at `target/release/ez-gui` (**19,908,688 bytes**, 19.9 MB / 19.0 MiB). The active desktop rebuild goal remains incomplete: literal SDR++ spacing/control parity and native window/RF/audio validation are still open. This report does not claim physical reception or audible playback was tested.

Only built-in Codex session subagents were used: `/root/integration_review`, `/root/meteor_completion`, and `/root/radio_rebuild`. They inherited the session model; the exact provider identifier was not exposed. Earlier explicit Astra dispatch had conflicting provider-prefix validation. No Morph or agy was used. Root identity: Codex harness / GPT-6. Existing unrelated dirty changes were preserved; nothing was committed or published.

## Completed work

- **Demodulation:** real DSB product detection and CW with a narrow RF filter plus adjustable beat tone; independent RF bandwidth and audio cutoff; corrected SSB sideband selection/pitch; streaming fractional resampling to the actual audio device clock; anti-alias filtering/persistent phase for optional RF decimation; streaming DC removal and repaired RF notch history.
- **WFM stereo:** pilot PLL, left/right recovery, linked AGC, per-channel audio processing/de-emphasis and resampling, smooth mono fallback, pilot indication, low-input-rate fallback, and consistent mono/stereo downmix loudness. Synthetic FM-IQ tests cover phase/frequency/carrier offsets, separation, pilot loss, weak pilot, chunk boundaries, and 44.1/48 kHz clocks.
- **RTL source:** background discovery/refresh, validated device selection using a unique USB serial when possible, explicit missing/ambiguous-device errors, real librtlsdr offset tuning, direct-sampling compatibility validation, and surfaced USB read errors. Demo timing now counts I/Q pairs, preserves submillisecond durations and subtracts generation time.
- **Audio output:** background device discovery, stable explicit output selection, supported sample-rate choice, F32/I16/U16 conversion, mono/stereo/multichannel routing, callback errors visible in the app, and a bounded 200 ms backlog that retains fresh complete frames. Changing source frequency/rate/type, output device, or mono/stereo format resets/drains audio before new samples.
- **Spectrum:** real streaming FFTs through 65,536 bins; configurable cadence; calibrated dBFS/window gain; bounded waterfall storage (8 MiB, at most 2048 columns) and bounded drawn traces; waterfall visibility; SNR smoothing; one-sided USB/LSB overlays; strongest-bin squelch within the actual tuned channel. Daemon-owned FFT size/window controls are disabled.
- **UI integration:** all eight local mode buttons; separate RF/audio controls; CW tone; mode-appropriate persisted snap intervals; stereo/pilot controls; device selectors; display controls; synchronized spectrum-menu/sidebar settings and Ctrl-scroll bandwidth edits. Audio DSP changes no longer reset spectrum settings. Receiver sample rate is saved on exit. Corrected translucent spectrum overlays, including the bright-orange WFM band found during visual review.

ADS-B/UAT and offline Meteor work from the previous pass remains intact. Meteor tests, including synthetic signed-CS8 recovery/export, passed in the integrated GUI suite. No recording controls were added to Meteor.

## Verification

- `cargo build -p ez-gui --release --features rtlsdr --offline` — passed; executable includes audio and local RTL-SDR.
- `cargo check --workspace --all-features --offline` — passed. Installed optional HackRF/Soapy pkg-config metadata is absent; their expected build warnings remain.
- `cargo check -p ez-gui --no-default-features --offline` — passed.
- GUI library suite — **553 passed, 0 failed, 1 ignored rendering test, 8 explicitly filtered socket tests**. Command: `cargo test -p ez-gui --lib --offline -- --skip daemon_client::tests --skip uat_receiver::tests::tcp_reconnect_and_cancellation_lifecycle --skip web_remote::tests::serve_index`.
- A preceding integrated run confirmed the two HTTP tests fail at socket bind with `Operation not permitted`; the other six excluded tests also require sockets. These exclusions are environment limits, not passing network evidence. No unrestricted workspace-test pass is claimed.
- Component evidence: **42 DSP**, **31 mode-helper**, **50 spectrum**, **29 source**, **12 audio-feature**, and **6 no-audio backend** tests passed. These overlap the integrated run and must not be added to its count.
- Six actual egui widget renders passed and were inspected at 1400×900 and 1000×700 for AM, WFM stereo controls, and CW. They are software-rendered tessellation, not native desktop screenshots or an SDR++ pixel comparison. Artifacts are in `artifacts/ui-review/radio*.png`.
- Touched Rust files pass rustfmt; `git diff --check` passes.

Logs: `/tmp/ez-parity-verified-gui.log`, `/tmp/ez-parity-verified-check.log`, `/tmp/ez-parity-release.log`, `/tmp/ez-parity-reviewed-render.log`, `/tmp/ez-parity-throughput.log`.

## Release throughput

The explicit `radio_performance` integration test processed one second of synthetic 2.4 MS/s I/Q, demodulation/resampling, and 65,536-bin FFT at 20 Hz. Every case produced exactly 48,000 audio frames.

| Mode | Processing time | Relative to input duration |
| --- | ---: | ---: |
| AM | 63 ms | 15.8× realtime |
| WFM mono | 105 ms | 9.5× realtime |
| CW | 61 ms | 16.5× realtime |
| WFM stereo | 146 ms | 6.8× realtime |

Command: `cargo test -p ez-gui --test radio_performance --release --features rtlsdr --offline -- --ignored --nocapture`.

This measures local CPU DSP/FFT throughput on this host. It excludes native rendering, USB, speaker callbacks, and end-to-end listening latency. It is not a whole-application frame-rate measurement.

## Remaining requirements

1. Obtain authoritative SDR++ source/native reference and compare equal-sized windows. Prior read-only clone escalation was rejected before execution because automatic approval review hit a model-routing 403. This continuation did not retry or bypass it. Literal toolbar/sidebar spacing remains unverified.
2. Complete remaining inventory: normal/center tuning modes, coordinated source IQ correction/inversion/decimation controls, direct-sampling I/Q branch selection, tone squelch/carrier AGC/IF noise reduction, audio sink streams, full waterfall-update option, and remaining installed module sections.
3. Verify native keyboard/mouse/rendering performance, actual USB shutdown/retuning, output device switching and audible stereo, and real RF reception. RTL discovery runs off-thread, but synchronous USB shutdown remains hardware-dependent.
4. Validate Meteor with an independent off-air recording. The synthetic CS8 fixture proves the software pipeline, not off-air robustness.
5. UAT978 still requires separately installed `dump978-fa`; it is a real decoded JSON-feed receive path, not an in-process UAT I/Q decoder. Daemon audio still lacks DSB/CW and stereo transport.

Detailed component reports: `DSP_PARITY_SUMMARY.md`, `STEREO_PARITY_SUMMARY.md`, `SOURCE_PARITY_SUMMARY.md`, `AUDIO_PARITY_SUMMARY.md`, `DISPLAY_PARITY_SUMMARY.md`. Current tracker: `RADIO_PARITY_TASK.md`; overall tracker: `UI_REBUILD_TASK.md`; reference inventory: `SDRPP_UI_REFERENCE.md`.
