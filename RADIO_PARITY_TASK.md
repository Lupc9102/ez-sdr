# Radio parity continuation

## Target
Continue the active EZ-SDR desktop rebuild without losing existing changes. Close real receiver gaps against installed SDR++: DSB/CW demodulation and independent channel bandwidth; RTL device selection and source options; complete FFT sizes and display controls. Integrate working UI controls, verify signal behavior and rendered layouts, then record remaining native/reference verification gaps. Exact SDR++ spacing remains a requirement, not an assumed result.

## Tasklist
- [x] Recover active goal, existing changes, and previous evidence — Codex harness / GPT-6 (root; exact provider route not exposed)
- [x] Implement and test DSB/CW, independent RF channel bandwidth, exact audio clocks and corrected RF decimation — native Codex session subagent `/root/integration_review` / inherited GPT-6 route; signal-level tests pass
- [x] Implement and test RTL enumeration/selection and offset tuning — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; 29 source tests including corrected Demo pacing and offline RTL feature check pass
- [x] Implement and test FFT sizes, display cadence, waterfall toggle, SNR smoothing and tuned-channel squelch measurement — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; 49 spectrum tests pass in isolated actual-source harness; full integrated check pending stereo/audio APIs
- [x] Integrate receiver controls, persistence, and DSP/source behavior — Codex harness / GPT-6 root; built-in display review findings fixed and integrated 553-test GUI run passes within documented socket exclusions
- [x] Implement audio output device/rate selection and integrate receiver UI — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route plus Codex root / GPT-6; 12 audio/6 no-audio backend tests pass
- [x] Implement WFM stereo decoding, pilot fallback, channel-aware output and signal-level verification — native Codex session subagents `/root/integration_review` and `/root/meteor_completion` / inherited GPT-6 route plus Codex root / GPT-6; 42 DSP tests pass, including FM-IQ stereo separation, pilot offsets/loss, low-rate fallback, and exact sample counts
- [x] Run focused checks, RTL release build, release throughput, and render/inspect actual widgets — Codex harness / GPT-6 root; 19.9 MB executable, 6.8–16.5× local DSP/FFT realtime, six reviewed software renders; native hardware/window validation remains in the overall tracker
- [x] Update authoritative parity record and standalone session summary — Codex harness / GPT-6 root; RADIO_PARITY_SESSION_SUMMARY.md and SDRPP_UI_REFERENCE.md updated

## Tips
- Integration checkpoint: root wired DSB/CW, independent RF/audio bandwidth, RTL async selection/refresh, offset tuning and display controls. `cargo check -p ez-gui --all-features --offline` passes with these APIs. Radio squelch defaults off, matching installed profile. Built-in source agent next owns audio_output.rs only; root integrates Audio selection.
- Pickup 2026-09-23: Codex harness / GPT-6, root. Exact provider identifier is not exposed. Prior explicit Astra dispatch encountered conflicting provider-prefix validation; inherited built-in session subagents work. User forbids Morph/agy and requests built-in Astra session subagents.
- Preserve the extensive pre-existing dirty tree. Do not reset unrelated work. Existing implementation and prior evidence are summarized in UI_SESSION_SUMMARY.md and SDRPP_UI_REFERENCE.md.
- Root owns radio_ui.rs, app.rs, config.rs and shared integration. Subagent DSP owns demod.rs and sdr_panel.rs. Source agent owns source_manager.rs. Display agent owns spectrum.rs. Each subagent uses its own task/summary file; send root findings and required API integration.
- Native X/audio/RF and independent off-air Meteor verification remain outstanding. Software-rendered egui widgets and synthetic signal tests must be labeled accurately.
