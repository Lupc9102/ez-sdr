# Claude Code "Skill Infinity" — Direct Execution vs Subagent Delegation

## Overview
The "skill infinity" pattern from CLAUDE.md prescribes an autonomous 24/7 daemon running an infinite Observe→Plan→Execute→Validate loop. This file documents both approaches.

---

## Comparison

| Dimension | Direct Execution (this session) | Subagent Delegation (Task tool) |
|---|---|---|
| **Context window** | Full, shared across loops — accumulates knowledge | Per-agent fresh/cleared per invocation — loses cross-loop memory |
| **Latency** | Zero overhead — tool calls happen immediately | ~5–15s per subagent launch + task setup |
| **State continuity** | Perfect — sees previous results, git state, build artifacts | Each subagent starts cold — must re-explain project context |
| **Parallelism** | Sequential (one thing at a time) | Trivially parallel — spawn N agents concurrently |
| **Error propagation** | Failures visible instantly, can self-correct | Subagent failures siloed — main loop may not detect |
| **Code modification** | Direct edits with full workspace awareness | Must re-read files; risk of stale context |
| **Validation chain** | Tight — build, test, lint in same process | Loose — subagent finishes, main loop must re-validate |
| **Complex task breakdown** | Manual — must plan and sequence each step | Automatic — subagent can self-organize sub-steps |
| **Communication overhead** | None | Must serialize results back to main loop |

## Recommendation

**Use Direct Execution for:**
- Code quality enforcement (lint fixes, type errors, formatting)
- Sequential validation pipelines (build → test → lint → commit)
- Tight feedback loops where context matters (e.g., fixing one bug, re-running, checking)
- Operations requiring workspace-wide awareness (refactoring, renames)

**Use Subagent Delegation for:**
- Embarrassingly parallel work (e.g., "run these 6 test suites simultaneously")
- Deeply independent research tasks (e.g., "audit this crate's security advisories")
- Long-running or risky experiments (isolate crash risk away from main loop)

## Session Status (2026-07-04)

### What was done
1. **Observe:** Ran `cargo check`, `cargo clippy -D warnings`, `cargo test` (839 tests, all pass)
2. **Plan:** Identified `unwrap()` calls flagged by clippy's `unwrap_used` lint in `dump1090/src/mode_ac.rs` and `dump1090/src/sdr/ifile.rs`
3. **Execute:** Replaced 19 `unwrap()` → `expect()` with descriptive messages; added `path_to_str()` helper
4. **Validate:** `cargo check`, `cargo clippy -D warnings`, `cargo test` (839/839 pass), `cargo fmt --check` — all green
5. **Commit:** `f008b89` — "lint: replace unwrap() with expect() in dump1090 tests for clippy conformance"
6. **Graph update:** `graphify update .` — rebuilt graph (1757 nodes, 3531 edges, 87 communities)

### What was done on this loop (2026-07-04)
1. **Observe:** `cargo clippy -D warnings` (1 warning: `duplicated_attributes`), `cargo test` (620 pass)
2. **Plan:** Fix remaining `unwrap()`/`unwrap_err()`/`panic!()` lints across ez-gui and dump1090
3. **Execute:**
   - Removed redundant `#![cfg(test)]` from `test_helpers.rs` (fixes `duplicated_attributes`)
   - Replaced 5 `panic!()` in `web_remote.rs` test match arms → `matches!()` with guard expressions
   - Replaced 4 `unwrap_err()` in `tle_engine.rs` → `expect_err()`
   - Replaced 5 `unwrap()` on `Option` in `dump1090/track.rs` test code → `expect()`
4. **Validate:** `cargo fmt --check`, `cargo clippy -D warnings -W clippy::unwrap_used -W clippy::panic`, `cargo test` (620/620 pass) — all green
5. **Graph update:** `graphify update .` — rebuilt graph (1766 nodes, 3539 edges, 88 communities)

### What was done on this loop (run 2, 2026-07-04)
1. **Observe:** Expanded clippy scope to redundant_clone, must_use_candidate, missing_panics_doc, missing_errors_doc
2. **Execute:**
   - Removed redundant `.clone()` calls in `app.rs`, `audio_output.rs`, `sdr_panel.rs`
   - Removed redundant `&` reference in `format!()` in `app.rs`
   - Converted `match` to `?` operator in `dump1090/fifo.rs`
   - Refactored late initialization to `let-if` expression in `satellite_panel.rs`
   - Added 12 tutorial tests (coverage for all 4 user levels + dispatch + data integrity)
   - Simplified `Result::map().unwrap_or()` → `Result::map_or()` in `adsb_decoder.rs` and `recorder_panel.rs`
   - Simplified `Result::map().unwrap_or_else()` → `Result::map_or_else()` in `recorder_panel.rs`
   - Added `# Panics` and `# Errors` doc sections to `crc.rs`, `fifo.rs`, `demod.rs`, `net_io.rs`, `sdr/mod.rs`
   - Added `#[must_use]` to constructors in `hackrf.rs`, `rtlsdr.rs`, `soapy.rs`, `dump1090/main.rs`
3. **Validate:** `cargo fmt --check`, `cargo clippy` with 10+ lint groups, `cargo test` (630/630 pass) — all green
4. **Commits:** `5eb36c9`, `0bd8ed6`, `b6ec46b`, `e39f260`, `6dd32be`

### Remaining work for next loop
- Project is now clean under `-D warnings -W clippy::unwrap_used -W clippy::panic -W clippy::must_use_candidate -W clippy::missing_panics_doc -W clippy::missing_errors_doc`. No remaining lints.
- 411 ez-gui tests + 219 dump1090 tests = 630 total, all passing.
- Full doc coverage for panics and errors in dump1090 public API.
- Next: continue deeper code quality — unused code removal, test coverage expansion, error handling improvements.
