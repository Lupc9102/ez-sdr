# Capture center and source tuning completion

Harness/model: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider identifier not exposed. No Morph or agy. Changed only `ez-gui/src/source_manager.rs` plus this source task/report pair.

## Implemented API

- `frequency_hz` remains the logical tuned/displayed VFO frequency.
- `center_frequency_hz: Option<u64>` defaults to None, following the VFO; Some selects an independent logical capture center.
- `capture_center_frequency_hz() -> u64` returns that effective center.
- `frequency_offset_hz: i64` defaults to zero. For local RTL only, physical tuner Hz = logical capture-center Hz + signed offset. Negative values subtract a transverter LO.
- `rtl_center_frequency_hz() -> Result<u32, String>` checks signed addition/subtraction and the librtlsdr 32-bit range before opening hardware. Logical RF frequencies above 4 GHz remain valid when their translated physical tuner frequency fits.
- `DirectSamplingBranch::{I,Q}` derives serde/default/equality, defaults to Q, and offers `label()`/`mode()`. `direct_sampling_branch` supplements the existing saved-compatible `direct_sampling` boolean. `direct_sampling_mode()` returns 0 when disabled, 1 for I, and 2 for Q.

The RTL worker receives the validated physical capture center and applies the exact enabled/disabled direct-sampling branch before tuning. Frequency translation never affects daemon, Demo, or file replay sources.

`tune_and_restart` is an explicit acquisition retune: it assigns both the VFO and `Some(capture center)` to the requested frequency before applying the sample rate and restarting or synchronizing the source. This prevents ADS-B mode changes from retaining an earlier independent capture center. Ordinary VFO-only changes remain the responsibility of app reconciliation.

`stream_generation() -> u64` exposes a private zero-initialized lifecycle counter. It increments once when a local worker or valid daemon connection attempt is installed, including same-settings restarts. `start()` is idempotent while either Running or Opening. Stopping, synchronous validation failures, and VFO-only changes preserve the generation. Root can include the generation in its capture signature to reset IQ mixer/FIR/FFT state after gain, offset, or replay-file restarts at an unchanged center and rate.

## Daemon behavior

Hardware SetFrequency commands use the logical capture center, with zero-centered full-capture spectrum subscriptions. Audio Subscribe/Retune commands use VFO minus capture center. Changing the VFO inside an independent capture changes the audio channel without moving hardware. Hardware status events update an independent center while retaining the VFO; center-following mode preserves its previous hardware-following behavior.

Audio tracking includes both absolute VFO and relative offset. This matters because the daemon channelizer preserves absolute channel frequencies when hardware retunes: center-following frequency changes now explicitly retune audio even when the offset remains zero. An unrepresentable channel offset unsubscribes audio and reports an error rather than wrapping to another frequency.

The existing daemon audio slice remains 200 kHz, capped by capture sample rate. Its full slice must fit inside the capture; native daemon range validation remains authoritative. This change does not expand daemon demodulation capabilities or alter ADS-B/Meteor data paths.

## Verification

- `cargo test -p ez-gui --offline --no-default-features source_manager::tests --lib`: **43 passed**. Regressions cover center-following/independent VFO, explicit acquisition retuning and worker replacement, >4 GHz translated RF, signed arithmetic boundaries including i64::MIN/u64::MAX, preflight errors, non-RTL offset isolation, I/Q branch modes/serde, actual generated daemon control/subscription/retune commands, hardware-state echo prevention, and overflow cleanup. Stream-generation coverage checks same-settings restart, stop stability, local/daemon idempotence including Opening, validation rejection, and VFO-only changes. Previous queue, replay, USB-selection, discovery, pacing, and cancellation tests remain passing.
- `cargo check -p ez-gui --offline --features 'audio rtlsdr'`: passed.
- Source file formatted with rustfmt.
- Read-only recording review confirmed daemon raw recording uses its wideband capture center (or capture center plus a channel offset); the full-capture spectrum subscription has offset zero. No daemon recording change was needed.

These are deterministic command/configuration tests and feature compilation. No physical USB device, native audio output, or live daemon server was exercised; daemon lifecycle coverage uses a failed loopback connection attempt. Actual RF tuning and USB shutdown remain hardware validation work.
