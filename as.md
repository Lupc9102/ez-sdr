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

## Missing Test Coverage

- [ ] `dump1090/src/sdr/soapy.rs` — SoapySDR device interface. Needs hardware to test, but basic constructor/factory tests possible with mocking.
- [ ] `dump1090/src/sdr/rtlsdr.rs` — RTL-SDR device interface. Gated behind `#[cfg(feature = "rtlsdr")]`. Needs hardware or librtlsdr dev package.
- [ ] `ez-gui/src/adaptive.rs` — Only `count_loud_samples` is tested (5 tests). The main `AdaptiveThreshold` struct with its gain adjustment logic (300+ lines) is untested.
- [ ] `ez-gui/src/recorder_panel.rs` — `free_disk_space_with_timeout()` (line 759) is untested.
- [ ] `ez-gui/src/web_remote.rs` — `RemoteCommand` enum and `WebRemote` struct untested. Network-heavy but constructor/serialization could be tested.
- [ ] `ez-gui/src/sdr_panel.rs` — Large UI module. `DemodMode` enum (from_label, etc.) could be tested in isolation.
- [ ] Large UI modules without unit tests: `app.rs` (3020), `ai_panel.rs` (1412), `adsb_panel.rs` (1640), `satellite_panel.rs`, `discord_panel.rs`, `howto_panel.rs` — these are egui-heavy, hard to unit-test without integration framework.

## Technical Debt / Cleanup

- [ ] `ez_sdr_config.json` — tracked in git with `web_remote_enabled: true` (non-default). Add to `.gitignore` or reset to defaults.
- [ ] `ez-gui/src/airport_db.rs` — `#[allow(clippy::type_complexity)]` on `FALLBACK_AIRPORTS` (line 467). Define a struct instead of the 9-element tuple.
- [ ] `graphify-out/2026-*/` dirs — session-specific graph backups. Add to `.gitignore` if not already covered.
- [ ] `Cargo.lock` — check for outdated dependencies with `cargo outdated` (needs `cargo-install outdated`).
- [ ] `rust-toolchain.toml` — channel `1.83` may be too low if any features need newer Rust. Verify against MSRV.

## Documentation

- [ ] Public API docs — many structs and methods still lack doc comments (`//!` or `///`). Focus on `discord.rs`, `source_manager.rs`, `spectrum.rs`, `sdr_panel.rs`, `config.rs`.
- [ ] README.md — build instructions are good. Could add test commands and CI badge once CI runs.

## CI / Infrastructure

- [ ] GitHub Actions badge — add `[![CI](https://github.com/…)](…)` to README.md once CI is running on a public repo.
- [ ] `cargo audit` — security vulnerability scanning. Add as a CI job or document as a manual step.
- [ ] Release profile — add LTO/strip to `Cargo.toml` for smaller release binaries.
