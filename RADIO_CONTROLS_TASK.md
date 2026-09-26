# Remaining Radio control parity

## Target
Continue the full native Rust application goal with the controls actually present in the installed SDR++ radio module. Establish exact installed control/mode evidence, implement missing functional radio DSP/control paths, integrate persistent controls, and validate signal behavior, rendering and release performance. Keep ADS-B map/region/plane SVG and offline Meteor CS8 work intact. Do not claim literal spacing or physical RF completion without direct evidence.

## Tasklist
- [x] Revalidate preceding turn as progress against current files and reports — Codex harness / GPT-6 root; exact provider route not exposed
- [x] Inspect installed SDR++ radio control/mode evidence and remaining layout gaps — Codex built-in radio_rebuild / inherited session model (provider route unavailable)
- [x] Implement missing verified carrier/IF/audio radio DSP controls with signal tests — Codex built-in integration_review / inherited session model (provider route unavailable)
- [x] Implement additional verified squelch/control processing with signal tests — Codex built-in meteor_completion / inherited session model (provider route unavailable)
- [x] Implement verified RDS controls with a real multiplex decoder and CRC/waveform tests — Codex built-in radio_rebuild / inherited session model (provider route unavailable)
- [ ] Integrate persistent mode-appropriate controls and validate complete radio path — root
- [ ] Render/review affected widgets, verify release and record remaining original-goal gaps — root
- [ ] Write standalone session report and update parity inventory — root

## Tips
- Pickup continuation 2026-09-23: Codex built-in session harness / GPT-6 root (exact provider route unavailable); finish integration alongside SHIP_READINESS_TASK.md audit workers.
- Pickup 2026-09-23: Codex harness / GPT-6 root. Use built-in session agents only. Explicit Astra selection failed provider routing (unprefixed rejected403; prefixed codex/gpt-6-astra rejected by tool model validation); working inherited session agents are available. No Morph/agy.
- Root owns app.rs/radio_ui.rs/config.rs/advanced_panel.rs integration. Assign file ownership before parallel edits; preserve the dirty worktree.
- Previous baseline: TUNING_PARITY_SESSION_SUMMARY.md. GUI590 pass with8 socket exclusions; release19,905,576bytes; full IQ→FFT→VFO→demod throughput71–182ms per synthetic second. Native/source-reference access remains limited.
- Current local Radio path is float source IQ → pre-mixer FFT → independent VFO → exact-rate complex demod. Raw recording/ADS-B use original byte/rate path. Legacy demod DC/decimation are disabled by app integration. Source stream_generation detects restarts, not EOF.
- Recheck actual installed SDR++ labels/config/symbols before interpreting earlier inventory entries. Do not invent unsupported reference controls or claim identical algorithms from names alone.
- Reference audit recovered exact four-column/two-button layout: visual rows NFM/AM/USB/LSB and WFM/DSB/CW/RAW. Common control order bandwidth/snap/de-emphasis/squelch/blanker/IFNR/highpass, then mode-specific controls, then Received Tone. Root owns persistent per-mode profiles. RDS owner radio_rebuild owns only new radio_rds.rs; demod owner supplies optional raw multiplex tap.
