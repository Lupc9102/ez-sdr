# EZ-SDR UI continuation task

## Target

Resume the EZ-SDR SDR++-style Rust UI work from the existing dirty worktree. Check the external Longcat/Claude swarm state, preserve existing changes, validate the current radio/ADSB/Meteor implementation, and finish any concrete gaps needed for a lightweight, usable app. Do not reset or clean unrelated work. The final state must be buildable and the session must leave a standalone summary document.

## Tasklist

- [x] Check the Longcat/Claude swarm and record whether any workers remain active: no Longcat/Claude/agy worker processes were found; only the current Codex sandbox was present. (Codex / GPT-6)
- [x] Inspect current task notes and diff for unfinished or accidental changes; existing work is intentional and the remaining parity gaps are explicitly tracked in `UI_BUILD_TASK.md`. (Codex / GPT-6)
- [ ] Run the full relevant EZ-SDR test/build checks and fix concrete regressions. (Codex / GPT-6)
- [ ] Review the final UI/runtime state and document limitations and verification results. (Codex / GPT-6)
- [ ] Write the standalone session summary. (Codex / GPT-6)

## Tips

- Existing work is intentionally dirty and includes broad prior SDR, ADS-B, Meteor, and UI changes; never reset or clean the worktree.
- `UI_BUILD_TASK.md` and `VISUAL_PARITY_AUDIT_SESSION_SUMMARY.md` contain the latest visual audit and parity findings.
- Longcat/Claude processes should be checked with `ps`; no external worker process should be assumed active without evidence.
- Physical SDR reception, audio hardware, native window interaction, and real off-air Meteor decoding remain environment-dependent.
- Harness/model: Codex / GPT-6 (current continuation agent), 2026-09-26.
