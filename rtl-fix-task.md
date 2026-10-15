# RTL-SDR Blog V4 Regression Fix

## Target

Fix only the RTL-SDR regression documented in `rtl.md`: enable RTL-SDR support in
normal `ez-gui` builds, identify the connected RTL-SDR Blog V4 reliably, feed live
IQ into the existing spectrum/waterfall path, and restore the AudioOutput lifecycle.
Preserve unrelated worktree changes and do not add SDR++ parity features.

## Tasklist

- [x] Audit the current app, source manager, and audio/spectrum APIs. (Codex / gpt-6)
- [x] Enable the RTL-SDR feature by default and improve Blog V4 selection/detection. (Codex / gpt-6)
- [x] Restore SpectrumAnalyzer batching and AudioOutput start/stop lifecycle. (Codex / gpt-6)
- [x] Run focused formatting, checks, tests, release build, and hardware smoke verification. (Codex / gpt-6)
- [x] Mark the user goal complete after the targeted fix is verified. (Codex / gpt-6)
- [x] Eliminate the periodic live-audio underrun caused by UI cadence and shallow worker queues. (Codex / gpt-6)

## Tips

- Picked up by Codex / gpt-6 on 2026-09-26.
- Keep the existing dirty changes in `ez-gui/src/app.rs` and
  `ez-gui/src/source_manager.rs`; inspect before editing and do not reset the tree.
- `SpectrumAnalyzer` already owns its worker and ring buffer. Do not add a second
  `SpectrumWorker` path.
- `AudioOutput` is the lifecycle wrapper around `AudioWorker`; initialize it lazily
  when playback starts so a missing audio device does not prevent radio startup.
- Connected hardware is an RTL-SDR Blog V4 (`0bda:2838`, serial `00000001`,
  manufacturer `RTLSDRBlog`, product `Blog V4`) and standalone `rtl_test`/`rtl_adsb`
  have succeeded.
- Follow-up picked up by Codex / gpt-6 on 2026-09-26 after manual testing found a
  very slight periodic audio skip. The local RTL read block is 8 ms at 2.048 MS/s;
  the 33 ms repaint interval with a four-block drain underfeeds audio over time.
  Keep the fix focused on scheduling and bounded buffering.
- Audio skip fix completed by Codex / gpt-6 on 2026-09-26: active repaint is 16 ms,
  local source drain is eight blocks, the demod queue is 16 frames, and the CPAL
  audio queue is 32 frames. Focused audio tests (12) and demod tests (67) pass;
  the release build is running as PID 303762 on display :1 for sustained playback
  testing.
