# Infinity Loop — Remaining Work

## Dead Code / Lint Suppressions to Clean Up

_All items below resolved — no `#[allow(dead_code)]` annotations remain in the listed files, and `RUSTFLAGS="-W dead_code" cargo check` produces zero warnings._

- [x] `ez-gui/src/adsb_panel.rs` — `#[allow(dead_code)]` removed; `show_map` field and `ui()` method no longer exist.
- [x] `ez-gui/src/app.rs` — `#[allow(dead_code)]` removed; Tab enum simplified, Recorder/Bookmarks/etc. moved to `SecondaryTool` and actively used.
- [x] `ez-gui/src/discord.rs` — `#[allow(dead_code)]` removed; `NotifKind::color` is now read in `discord_panel.rs`.
- [x] `ez-gui/src/airport_db.rs` — `#[allow(dead_code)]` removed; `AntennaDims` struct no longer has `freq_mhz` field.
- [x] `ez-gui/src/tle_engine.rs` — `#[allow(dead_code)]` removed; `los_dt` and `doppler_shift()` are used by scheduler/app/satellite modules.
- [x] `ez-gui/src/adsb_decoder.rs` — `#[allow(dead_code)]` removed; no dead fields remain.
- [x] `ez-gui/src/demod.rs` — `#[allow(dead_code)]` removed.
- [x] `ez-gui/src/theme.rs` — `#[allow(dead_code)]` removed; only `clippy::type_complexity` suppression remains (addressed under Technical Debt).

## Stub / TODO Implementations

- [x] `dump1090/src/mode_s.rs` — `decode_mode_s_message()` fully implemented: handles DF17/18 callsign, altitude, velocity; DF0/4/5/16/20/21 altitude; CRC validation. 11 tests pass.
- [x] `dump1090/src/cpr.rs` — Airborne decode test verified against dump1090 reference implementation; values match to within tolerance. Test passes.
- [x] `dump1090/src/mode_ac.rs` — stub replaced with full implementation; round-trip tests pass.

## Missing Test Coverage

- [x] `dump1090/test` coverage — `fifo`, `stats`, `track`, `demod`, `ifile`, `convert`, `net_io`, `icao_filter` all have tests.
- [x] `dump1090/src/sdr/soapy.rs` — SoapySDR device interface. Added constructor/factory tests (`SoapyConfig::default()`, `SoapySdr::new()`, Drop with null pointers, Send impl). Feature-gated behind `#[cfg(feature = "soapy")]`.
- [x] `dump1090/src/sdr/rtlsdr.rs` — RTL-SDR device interface. Added constructor/factory tests (`RtlSdr::new()` with/without device name, Drop with null device, Send impl, gains is empty). Feature-gated behind `#[cfg(feature = "rtlsdr")]`.
- [x] `ez-gui/src/adaptive.rs` — 17 new tests for AdaptiveThreshold (commit b0d3189).
- [x] `ez-gui/src/recorder_panel.rs` — 7 new tests including free_disk_space (commit b0d3189).
- [x] `ez-gui/src/web_remote.rs` — 8 new tests for RemoteCommand, set_enabled, etc. (commit b0d3189).
- [x] `ez-gui/src/sdr_panel.rs` — 12 new tests for DemodMode, identify_frequency, etc. (commit b0d3189).
- [x] `ez-gui/src/spectrum.rs` — 29 tests for color pipeline (lerp_color, sample_palette, color_map, waterfall_color_classic, WindowType::generate). Commit b4297fa.
- [x] `ez-gui/src/discord.rs` — 30+ tests for categories(), kinds_in(), is_enabled(), is_starred(), all 13 embed builders. Commits fd5a022 & 4d85771.
- [x] `ez-gui/src/scanner.rs` — 17 tests for parse_json helpers. Refactored into generic parse_json_value<T>. Commit fd5a022.
- [x] `ez-gui/src/app.rs` — 9 tests for parse_hhmm_today_at (timezone bug fixed). Commit 52fd9d1.
- [ ] Large UI panel modules: `ai_panel.rs` (1412), `adsb_panel.rs` (1640), `satellite_panel.rs`, `discord_panel.rs`, `howto_panel.rs` — egui-heavy, need integration harness.

## Technical Debt / Cleanup

- [x] `ez_sdr_config.json` — tracked in git with `web_remote_enabled: true` (non-default). Already in `.gitignore`; removed from git tracking via `git rm --cached`.
- [x] `dump1090/src/config.rs` — `items_after_test_module` Clippy lint fixed.
- [x] Unused `libc` dependency — removed from `Cargo.toml`.
- [x] `ez-gui/src/airport_db.rs` — 9-element `AirportEntry` tuple struct replaced with named struct; inner frequency tuples replaced with `FallbackFreq` named struct.
- [x] `graphify-out/2026-*/` dirs — already in `.gitignore` (line 11).
- [ ] `Cargo.lock` — `cargo outdated` not installed; skipped (install with `cargo install cargo-outdated`).
- [x] `rust-toolchain.toml` — already on channel `1.92` (not `1.83` as previously noted). Verified: `cargo check`, `cargo clippy`, `cargo test` all pass cleanly.

## Documentation

- [x] Public API docs — all public items in `discord.rs`, `source_manager.rs`, `spectrum.rs`, `sdr_panel.rs` already have complete `///` doc comments. Verified no missing public item docs in these files.
- [x] `SAFETY` comments — added to all 78 `unsafe` blocks in SDR modules (`dump1090/src/sdr/`).
- [x] README.md — build instructions are good. Test commands and CI badge added.

## CI / Infrastructure

- [x] GitHub Actions badge — `[![CI](https://github.com/Lupc9102/ez-sdr/actions/workflows/ci.yml/badge.svg)](…)` added to README.md.
- [x] `cargo audit` — security vulnerability scanning. Added as a CI job.
- [x] Release profile — LTO/strip already configured in workspace `Cargo.toml`.
