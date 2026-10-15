# ez-sdr Functionality and Web-Reference Audit

## Target
Perform a deep, evidence-based audit of every user-facing and internal ez-sdr function, with special focus on ADS-B decoding, Meteor LRPT decoding, SDR hardware behavior, demodulation modes, satellite workflows, and every radio-frequency claim. Trace functionality from UI/API entry points through runtime implementations, identify broken or misleading behavior, inventory all external web references and uncited technical claims, and verify those claims against authoritative primary documentation. Correct defects and documentation gaps where the evidence supports a concrete fix, run focused and repository-wide validation, and produce a standalone final audit report with source URLs, file/line evidence, limitations, and prioritized remaining risks.

## Tasklist
- [x] Inventory repository surfaces, features, runtime paths, frequency presets, URLs, and technical claims. — Codex native / GPT-6 (direct current-tree searches and graphify query; external inventory still running)
- [x] Trace and audit ADS-B functionality — Codex native / GPT-6 against authoritative Mode S / ADS-B documentation.
- [x] Trace and audit Meteor LRPT decoding — Codex native / GPT-6 against authoritative protocol, spacecraft, and decoder documentation.
- [x] Audit all other ez-sdr modes, device support, frequency claims, and web references against authoritative sources.
- [x] Correct verified code, UI, workflow, test, and documentation defects within scope.
- [x] Run targeted tests and repository-wide validation; record evidence and limitations.
- [x] Produce the standalone functionality and web-source audit report. — Codex native / GPT-6
- [x] Produce the standalone session summary document. — Codex native / GPT-6

## Tips
- Active agent: harness=Codex native, model=GPT-6 (2026-09-18).
- Active external agent: harness=Codex-agy, model=Gemini 3.6 Flash high reasoning; repository structure and feature inventory (2026-09-18).
- Active external agent: harness=Codex-agy, model=Gemini 3.6 Flash high reasoning; ADS-B/Mode S documentation audit (2026-09-18).
- Active external agent: harness=Codex-agy, model=Gemini 3.6 Flash high reasoning; Meteor LRPT documentation audit (2026-09-18).
- Active external agent: harness=Codex-agy, model=Gemini 3.6 Flash high reasoning; frequencies, modes, devices, and web-source audit (2026-09-18).
- Treat the current worktree as authoritative; it contains substantial uncommitted work from an earlier audit and must not be overwritten or reverted.
- Prefer standards bodies, government/space-agency material, chipset/vendor manuals, and upstream protocol implementations over blogs or frequency-list aggregators.
- For each claim, distinguish protocol requirements, regional allocations, common operating practice, and hard-coded application policy.
- Existing `graphify-out/` data is an orientation aid only; verify every conclusion directly against current files and current external documentation.

- Follow-up audit (Codex native / GPT-6, 2026-09-18): web LRPT now supports M2-3/M2-4; SSB oscillator uses input-rate timing; NOAA APT wording corrected.
