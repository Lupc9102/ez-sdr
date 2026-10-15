# Streaming broadcast RDS decoder

## Target
Implement `ez-gui/src/radio_rds.rs` only: bounded streaming57kHz WFM multiplex frontend, differential/biphase symbol decoding, CRC-validated26bit RDS block and group synchronization, group0 program service and group2 RadioText, truthful PI/PTY/TP/TA/music/region presentation. Root owns app/UI/config; DSP owner supplies raw multiplex. Prove complete synthetic waveform reception with unknown phase, noise and carrier offset, CRC corruption rejection, reset and station change. Do not invent metadata or claim hardware reception.

## Tasklist
- [x] Finalize installed Radio audit and agree raw multiplex API — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed)
- [x] Implement bounded protocol parser and metadata state — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed).
- [x] Implement streaming57kHz frontend and timing acquisition — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed).
- [x] Prove protocol integrity and complete waveform recovery with tests — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed); 8 optimized standalone tests pass; Cargo lib verification9/9 passed,0.93 seconds after peer-review fix.
- [x] Send integration API, finalize report and hand off module — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed).

## Tips
- Pickup2026-09-23: native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider identifier not exposed. Built-in session harness only, no Morph/agy.
- Read `RADIO_CONTROLS_TASK.md` and `RADIO_CONTROL_REFERENCE.md`. File ownership: this agent only radio_rds.rs plus scoped reports; parent root app/config/UI, integration_review demod.
- Demod API agreed: `set_rds_tap_enabled(bool)`, `take_wfm_multiplex()->(Vec<f32>,f64)`; raw FM discriminator normalized75kHz peak deviation, before deemphasis/audio LPF. WFM rate can be250kHz or source rate2.4MHz. Tap clears/replaces each demodulate/reset/non-WFM.

- Initial optimized standalone verification:6 tests passed, including full waveform57kHz demix→biphase differential→CRC groups→PS/RT at250000,240000.5 and2400000Hz with±3Hz carrier offsets, phase and noise. Root informed module/API ready; stronger shaped waveform/partition/reset tests next.

- Shaped waveform test exposed/fixed a real block-boundary issue: once synchronized, next A must follow D by26bits, rather than accepting coincidental A syndromes in intervening payload windows. RRC-shaped2375chip/s waveform,−12Hz carrier offset,+100ppm symbol-clock offset and2B RadioText pass. Root exports module and owns app/UI integration; DSP owner now owns full IQ→RDS integration/performance tests and received exact fixture guidance.

- Measured inline decoder state2016bytes. Peer asked for short read-only metadata/CRC review; root has ready-to-integrate API and Cargo passing result.

- Peer review by built-in session agent `/root/meteor_completion`: CRC/group sequencing and PS/RT masks reviewed cleanly. Fixed North American RBDS PI interpretation: country/coverage/reference helpers now return None there, preserving raw PI and RBDS PTY. Added regression fixture; final Cargo RDS result9 passed,0 failed,0.93 seconds. Full application IQ-to-RDS integration/performance remains assigned to integration_review.
