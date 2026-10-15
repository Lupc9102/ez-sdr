# Display parity implementation

## Target
Complete the real spectrum display capabilities required by the SDR++ parity continuation: power-of-two FFT sizes 256–65536, streaming IQ accumulation across short and long caller blocks, bounded transform cadence/work/memory, bounded waterfall texture storage, waterfall visibility, and SNR smoothing. Preserve existing spectrum visuals and interaction. Final visual QA also requires correcting straight RGBA constants passed as premultiplied colors, including the bright WFM RF overlay; retain intended palette/opacity values and scale already-premultiplied bookmark colors correctly. Root owns UI/configuration/app integration; this agent edits only `ez-gui/src/spectrum.rs` and this task/summary pair. Tests must verify signal/chunk/cadence behavior, not infer native UI/RF success.

## Tasklist
- [x] Inspect current FFT/waterfall/rendering paths and agree integration APIs with root — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed)
- [x] Implement validated large FFT streaming and bounded frame cadence — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed)
- [x] Implement bounded waterfall storage/texture aggregation and visibility — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed)
- [x] Implement SNR smoothing and expose working display APIs — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed)
- [x] Add and run meaningful signal, chunk-boundary, cadence, memory-bound and validation tests (49 spectrum tests pass through normal Cargo after integration; isolated harness also passed earlier) — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed)
- [x] Write DISPLAY_PARITY_SUMMARY.md with verified behavior and remaining limits — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed)
- [x] Disable source-owned FFT size/window menu controls when daemon supplies the spectrum; inspect the enabled scope and pass all 49 spectrum tests through normal Cargo — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed)
- [x] Correct straight RGBA construction and bookmark opacity scaling in spectrum.rs; emitted WFM overlay alpha/color regression and all 50 spectrum tests pass — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed)

## Tips
- Alpha-fix verification: all straight RGBA constants now use `from_rgba_unmultiplied`; the two bookmark waterfall dimmers use `gamma_multiply` to reach the existing 90/130 alpha values without re-premultiplication. Headless egui output confirms the WFM overlay contributes premultiplied RGB approximately 22/12/4 at alpha 22. Normal Cargo spectrum suite passed 50 tests; root owns the final screenshot rerender.
- Alpha-fix pickup: native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider model not exposed). Root's final WFM render shows opaque orange despite alpha 22; change color representation, not palette, then notify root to rerender.
- Final daemon-menu verification: `cargo test -p ez-gui --no-default-features --lib spectrum::tests -- --nocapture` passed (49 passed, 0 failed). FFT size/window controls share the disabled scope and tooltip; averaging and later display controls remain outside it. No native pointer interaction is claimed.
- Final daemon-menu pickup: native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (provider identity not exposed). Root sets `fft_controls_enabled` by source mode; scope only FFT resolution/window menu enablement and focused verification.
- Pickup 2026-09-23: native Codex session subagent `/root/radio_rebuild`, inherited GPT-6-family model route; exact provider identifier is not exposed. Read RADIO_PARITY_TASK.md and UI_REBUILD_TASK.md. No Morph or agy; no broad graph rebuild.
- Existing push_iq_samples zero-pads short blocks and transforms only the first FFT-size samples of long blocks; storage width equals FFT size and history allows4096rows. Both need replacement before exposing65536.
- Root owns radio_ui.rs/app.rs/config.rs; communicate APIs before implementation and do not edit those files.
