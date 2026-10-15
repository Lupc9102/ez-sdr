# Source and tuning continuation — 2026-09-23

## Outcome

Implemented independent Radio VFO tuning, floating-point source IQ processing, source offset/direct-sampling controls, and capture-correct spectrum and recording metadata. The original native-app goal remains active: this is verified progress, not a claim of literal SDR++ parity or physical receiver validation.

## Session agents and routing

All delegation used the built-in session agents: `/root/integration_review`, `/root/meteor_completion`, and `/root/radio_rebuild`. No Morph or agy was invoked. Root used the Codex harness / GPT-6 identity; working subagents inherited the session model, whose exact provider identifier is not exposed.

A fresh built-in spawn with `model: gpt-6-astra` was accepted, but execution failed with provider 403: the provider requires `codex/gpt-6-astra`. A second spawn using that exact prefixed name was rejected by the built-in tool as an unknown model. Existing built-in session agents continued the authorized work. No exact Astra identity is claimed for those inherited agents.

## Implementation

- Normal tuning moves a digital VFO inside the captured band without restarting acquisition. Center mode and hardware tuning beyond the useful band recenter acquisition. Replay tuning preserves its fixed capture center and playback position, with an explicit File center metadata control.
- The local Radio path is source bytes → float IQ/DC correction/inversion/filtered decimation → capture-centered FFT → independent complex VFO mixer → float mono/stereo demodulation. Intermediate IQ is not requantized. Exact fractional effective sample rates reach spectrum, mixer and audio resampling.
- ADS-B and raw recordings receive the original source bytes and source rate. ADS-B temporarily bypasses Radio IQ preferences without losing them. Meteor remains a separate offline decoder.
- Added Off/I/Q direct sampling, checked additive transverter offsets and installed SDR++ offset presets. Logical RF display supports the twelve-digit tuning range; physical RTL tuning is checked separately.
- Daemon hardware control uses capture center; audio subscriptions and retunes use VFO minus capture center. Local RTL converter offsets are not sent as daemon tuning offsets.
- Source generations distinguish new workers/connections, including same-settings restarts. Final replay buffers retain their filter, decimation and mixer state when EOF is consumed. Settings rate/gain changes restart acquisition where needed; remote gain applies through the source-settings path.
- Spectrum supports float complex samples, independent VFO overlay/detector, fractional rates and functional full/partial waterfall texture updates. The sample-rate readout preserves fractional MS/s values.
- IQ sidecars store capture center and original sample rate, plus separate tuned frequency and converter offset. Satellite recorder metadata now takes actual capture center/rate. This does not implement metadata segmentation during a recording that is retuned later.

## Validation

- GUI library suite: **590 passed**, zero failures, one ignored visual test, eight socket tests excluded. Command: `cargo test -p ez-gui --lib --offline -- --skip daemon_client::tests --skip uat_receiver::tests::tcp_reconnect_and_cancellation_lifecycle --skip web_remote::tests::serve_index`.
- `cargo check --workspace --all-features --offline` passed, with expected optional HackRF/Soapy library warnings. GUI check without default features also passed.
- Component coverage includes 45 demodulation, nine new IQ, 43 source and 57 spectrum tests. These overlap the GUI count and must not be added to it.
- Nonignored complete-path AM signal test passed at decimation 1/8, including noninteger effective rate, arbitrary odd byte chunks, two distinct RF carriers, selective demodulation and capture-centered FFT coordinates.
- Scoped integration review found and verified fixes for replay EOF state loss, replay relabel/rewind, stale worker rate after Settings changes and same-settings restart detection. Report: `TUNING_INTEGRATION_REVIEW.md`.
- Actual egui widget software renders were inspected at 1400×900 and 1000×700 for Radio/VFO, RTL source and Replay. Artifacts are in `artifacts/ui-review/tuning/`; these are not native window screenshots. RTL discovery is stubbed in unit-test builds, so the source preview's unavailable-build message is not a physical USB result.
- Formatting and `git diff --check` passed. Existing unrelated dirty changes were preserved; no commit/reset/revert was performed.

The eight-case release throughput test passed using source rate 2,400,003 Hz, +18 kHz VFO offset, source DC correction, a 65,536-bin FFT at 20 Hz, exact-rate complex demodulation, and decimation 1/8. Timed work includes IQ processing, FFT, mixing and demodulation; fixture generation is excluded. Each case produces 47,999 audio frames (48,000±1); both WFM stereo cases lock.

| Mode | Decimation 1 | Decimation 8 |
| --- | ---: | ---: |
| AM | 84 ms | 73 ms |
| WFM mono | 122 ms | 79 ms |
| CW | 84 ms | 71 ms |
| WFM stereo | 182 ms | 109 ms |

These are CPU signal-processing timings, not native frame rate, USB performance or audible latency. Command: `cargo test -p ez-gui --test radio_performance --release --features rtlsdr --offline -- --ignored --nocapture`. Exact component evidence is in `VFO_DISPLAY_SUMMARY.md`.

Final `cargo build -p ez-gui --release --features rtlsdr --offline` passed after all production edits. `target/release/ez-gui` is **19,905,576 bytes** (19.9 MB / 19.0 MiB), including audio and RTL support. Final formatting and whitespace verification passed.

Root logs: `/tmp/ez-tuning-gui-final.log`, `/tmp/ez-tuning-workspace.log`, `/tmp/ez-tuning-noaudio.log`, `/tmp/ez-tuning-release-final.log`, `/tmp/ez-tuning-render.log`, and `/tmp/ez-tuning-render-final.log`.

## Remaining original-goal gaps

Exact SDR++ upstream-source and equal-scale native render comparison remain unavailable; the earlier read-only upstream clone escalation was rejected because automatic approval review hit its own model-routing 403 before execution. No bypass was attempted. `SDRPP_UI_REFERENCE.md` distinguishes confirmed inventory from provisional spacing.

Unimplemented reference areas still include tone/CTCSS squelch, carrier AGC, IF noise reduction, RDS, audio-lowpass bypass, multiple sink streams and the complete installed module/sidebar inventory. Physical RTL/audio, native frame latency and daemon/socket behavior still need an unrestricted environment. UAT978 remains real dump978-fa decoded JSON ingestion, not a native IQ UAT demodulator. Meteor CS8 synthetic decoding/export was proven in the preceding work; an independent off-air capture remains unverified.

## Handoff

Start from `TUNING_PARITY_TASK.md`, `SDRPP_UI_REFERENCE.md`, `IQ_TUNING_SUMMARY.md`, `SOURCE_TUNING_SUMMARY.md`, `VFO_DISPLAY_SUMMARY.md` and this report. Respect the user's built-in-subagent preference and preserve the dirty workspace. Do not substitute successful software tests for literal layout or hardware evidence.
