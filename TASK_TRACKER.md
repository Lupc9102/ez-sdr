# EZ-SDR Unified App Task Tracker

## Target

Turn this SDR prototype into a coherent, beginner-friendly all-in-one application covering the intended dump1090, SDR++, and SatDump workflows. Read the repository guidance and README, audit the current implementation, fix the most consequential functional and usability gaps, and leave a detailed handoff plan for the next agent.

## Tasklist

- [x] Read README and repository structure — Codex / GPT-6 / functions.exec (2026-09-20)

- [x] Audit current application behavior and identify highest-impact defects — Codex / GPT-6 / functions.exec (2026-09-20)

- [x] Implement focused fixes that improve startup, core SDR workflow, and beginner usability — Codex / GPT-6 / functions.exec (2026-09-20)

- [x] Run available checks/tests and fix regressions — Codex / GPT-6 / functions.exec (2026-09-20)

- [x] Write detailed handoff plan for remaining work — Codex / GPT-6 / functions.exec (2026-09-20; `EZ_SDR_CONTINUATION_PLAN.md`)

- [x] Write session end summary — Codex / GPT-6 / functions.exec (2026-09-20; `EZ_SDR_SESSION_SUMMARY.md`)

## Tips

- Active agent: Codex GPT-6, harness: functions.exec / shell; started 2026-09-10.

- Keep scope evidence-driven: prioritize broken paths and usability blockers over speculative rewrites.

- Continuation agent: Codex GPT-6, harness: functions.exec / shell; resumed 2026-09-20 after a failed delegation attempt. Direct workspace inspection is authoritative; do not infer completion from prior audit prose.

## Continuation tasklist (2026-09-20)

- [x] Reconcile the five named audit reports with the current worktree and record concrete unresolved requirements. — Codex GPT-6 / functions.exec (2026-09-20)
- [x] Fix remaining ADS-B correctness gaps: GUI IQ block carry, short-message correction ordering, CPR normalization/runtime behavior, and filter lifecycle. — Codex / GPT-6 / functions.exec (2026-09-20)
- [x] Implement and validate LRPT receive-chain corrections: CCSDS Viterbi/RS, OQPSK hypotheses, receive-layer ordering, and MSU-MR JPEG reconstruction. — Codex / GPT-6 / functions.exec (2026-09-20; 65 tests pass)
- [x] Fix remaining LRPT/SDR DSP and UI claims: SSB sideband selection, AM/NFM anti-alias filtering, Doppler retuning, CW wording, hardware gain, and frequency overlays. — Codex / GPT-6 / functions.exec (2026-09-20)
- [x] Fix web/API mismatches such as unsupported vertical-rate display and explicitly document ADS-B protocol scope. — Codex / GPT-6 / functions.exec (2026-09-20)
- [x] Replace synthesized satellite positions, passes, and Doppler with parsed TLE/SGP4 propagation, current-file import, and Earth-fixed observer geometry. — Codex / GPT-6 / functions.exec (2026-09-20; 10 focused tests pass)
- [x] Run targeted and workspace validation; update this tracker and SESSION_SUMMARY.md with evidence. — Codex / GPT-6 / functions.exec (2026-09-20; Rust workspace, all-features check, web tests/build, formatting, and diff validation passed)

## Continuation tips

- Existing modifications are user work and must be preserved; inspect `git diff` before touching overlapping files.
- Prior audit reports claim several fixes that current-tree reconciliation may contradict. Verify source and tests directly.
- Completion agent: Codex / GPT-6, harness: functions.exec; SGP4 remediation, full validation, audit reconciliation, and session documentation completed 2026-09-20.
