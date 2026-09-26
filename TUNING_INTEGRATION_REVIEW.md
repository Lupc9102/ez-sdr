# Tuning integration review

Read-only review by native Codex session subagent `/root/integration_review`, inherited GPT-6 route, on 2026-09-23. The exact provider/model identifier is unavailable. Root reported explicit Astra routing failures; this review uses the existing built-in session agent. No Morph/agy and no source edits.

**Final outcome: all findings from this scoped review have been addressed and verified in the final source. The replay regression and two local generation tests pass. No unresolved finding remains from this review.**

Reviewed current `app.rs`, `radio_ui.rs`, `advanced_panel.rs`, and `config.rs`, with focused lifecycle/metadata context from `source_manager.rs`, `recorder_panel.rs`, `satellite_panel.rs`, and `adsb_panel.rs`. Root was concurrently fixing integration. Findings were sent promptly, then each reported fix was reread in the final source before closing this review.

## Resolved findings

### Fixed — EOF no longer resets processing before the last replay buffers

The original reviewed `CentralApp::logic` source drain collected up to four sample buffers and then computes the `receiving` flag from the current source status. `SourceManager::recv_samples` can encounter `EndOfStream` after collecting one to three valid final buffers and change the status to `Idle`. The subsequent `last_radio_capture` tuple comparison sees `receiving` change from true to false and resets `RadioIqProcessor`, `VfoMixer`, and `Demodulator` before processing those final buffers.

The final samples belong to the same capture and require the existing FIR/DC history, decimation remainder and oscillator phase. Resetting them breaks the streaming contract, can change the decimated sample count, and introduces a VFO/demodulation transient near the file tail. A fresh stream-generation counter should trigger reset at the next capture, while EOF/stop must not reset state before processing an already-drained tail. **Verified fix:** the capture identity is now `(center, source_mode, stream_generation)` and contains no `receiving` member. EOF and stop do not advance source generation; a new local worker or daemon connection does. Thus consuming EOF while collecting the final batch does not reset its DSP state. Audio identity also includes the generation.

### Fixed — Replay retains its capture center and playback position during tuning

The original reviewed `radio_ui::reconcile_tuning` applied capture-center changes and `restart_if_running` to Replay sources. For example, replay a file whose declared center is 118 MHz, then tune outside the current span to 120 MHz or enable Center mode with an offset VFO. The source center changes to 120 MHz and the replay worker restarts at byte zero. The file data still represents the original 118 MHz capture: the Replay worker does not implement a hardware retune or translate the recorded spectrum to the new center.

Within-span digital tuning works, but an out-of-span/Center operation now claims unavailable RF coverage and rewinds playback. Keep file capture center fixed during VFO tuning and constrain/reject targets outside the file's usable span. If manual metadata relabeling is desired, expose that as an explicit file-center setting rather than a tuner operation. **Verified fix:** Replay now returns from reconciliation before the hardware-center/restart path, preserves its declared center, and clamps VFO frequency to the usable recorded band. The Center toggle is disabled for Replay; a distinct File center field provides explicit metadata relabeling. The new regression verifies center, replay position, stream generation and running status remain unchanged across normal and out-of-span tuning, including a saved `center_tuning=true` preference.

## Other verified fixes

- **Settings sample-rate mismatch:** Settings `needs_apply` assigned `default_sample_rate`/gain then called `tune` with the default frequency. If the resulting VFO remained inside the useful capture band, the new tune helper did not restart the existing worker. DSP and metadata could therefore use 2.4 MS/s while the worker continued producing 2.048 MS/s. **Verified fix:** the final code compares old rate/gain, applies the requested tuning, then explicitly restarts only if settings changed without an already-triggered center restart.
- **Same-settings restart identity:** Existing capture/audio identity tuples lacked a generation identifier. Changing RTL device, gain, converter offset, replay file or speed can stop/start at the same center/rate/source mode; local start ends in `Running`, so the processing loop may never observe a stopped state. Old audio and DSP state could survive the restart. **Verified fix:** `stream_generation()` is implemented and included in both processing-capture and audio identities. It advances at new worker/connection creation, not stop or VFO-only edits; final source preserves generation on rejected/idempotent starts. Two focused local generation tests pass.
- **Remote gain:** `RemoteCommand::SetGain` assigned the gain and called the new tune helper with unchanged frequency, which does not restart a local worker. Historical baseline already failed to apply remote gain, so this was explicitly corrected to an existing limitation in a newly touched path, not an introduced regression. **Verified fix:** the final remote-gain branch calls `restart_if_running` directly after assigning gain.
- **Raw recording metadata:** Root changed local IQ and satellite recording metadata to actual capture center/original source rate, preserving tuned RF and converter offset separately for local IQ sidecars. The current read showed these fields selected from the source correctly. This does not prove metadata segmentation across retunes during an already-running recording.

## Paths checked without an additional finding

- The local spectrum receives floating-point source-processed IQ before VFO translation. Mono/stereo demodulation receives the separate complex mixer output and exact f64 output rate.
- Root disables the demodulator's legacy RF DC removal/decimation, avoiding double processing, while retaining its RF blanker/notch controls.
- Raw local recording, satellite recording, constellation input and ADS-B receive the original byte buffers. ADS-B receives the original source rate; enabling ADS-B bypasses Radio DC/inversion/decimation preferences without discarding those saved preferences.
- Normal local tuning retains capture center while the selected RF channel fits inside the filter's useful band. Decimation reduces the permitted span; USB/LSB reserve a one-sided width. Hardware converter offsets affect physical tuner frequency without changing the logical VFO offset formula.
- Daemon source configuration bypasses local IQ processing; daemon control synchronization uses capture center and its audio subscriptions use VFO-minus-center. Unsupported local DSB/CW modes are suppressed by source workflow logic. No network round-trip behavior was exercised.

## Verification and limits

- Reread final source for the EOF tuple, stream-generation lifecycle, audio restart identity, fixed Replay center/clamping, File center control, Settings rate/gain restart and remote-gain restart.
- `cargo test -p ez-gui --no-default-features --offline replay_tuning_preserves_file_center_and_playback_position --lib` — **1 passed**.
- `cargo test -p ez-gui --no-default-features --offline stream_generation --lib -- --skip daemon_stream_generation` — **2 passed**; the network-dependent daemon test was deliberately excluded from this focused local verification.

No source files were changed. This combines static integration review with the three focused regressions above, not native UI/hardware/network verification or a full GUI-suite claim. Root owns final integrated tests, release and throughput validation. Prior component evidence remains in `IQ_TUNING_SUMMARY.md`.
