# Side Menu Cleanup Task

## Target

Audit the application UI for side menus and tabs (including Discord and AI), decide which surfaces are not needed for the core radio workflow, remove low-value or redundant menus, and make retained utility surfaces open in compact independent pop-up windows instead of occupying the main split-screen layout. Preserve core radio controls, receiver views, and required operational status. Validate the changes with focused tests/build checks and record any remaining limitations.

## Tasklist

- [x] Map the UI entry points, side-menu surfaces, and existing window/panel abstractions. — Codex / GPT-6 (ui_map subagent)
- [x] Decide the keep/remove set and implement removal of low-value side menus. — Codex / GPT-6 (ux_decision)
- [x] Convert retained secondary tools to compact pop-up windows with usable sizing and close behavior. — Codex / GPT-6 (primary agent)
- [x] Run focused tests/build checks and inspect the final diff for regressions. — Codex / GPT-6 (primary agent)
- [x] Write the session summary with completed work and verification results. — Codex / GPT-6 (primary agent)

## Tips

- Active harness/model: Codex / GPT-6 (primary agent).
- Audit agent harness/model: Codex / GPT-6 (ux_decision subagent; mapped UI surfaces and keep/remove recommendations).
- Audit agent harness/model: Codex / GPT-6 (ui_map subagent; entry-point and panel abstraction inventory).
- UI-map notes (Codex / GPT-6): `CentralApp::ui` in `ez-gui/src/app.rs` owns all top-level layout. `active_secondary_tool` currently renders `Panel::left("secondary_panel")` (360 px) and `ai_ask_open` renders `Panel::right("ai_ask_panel")` (340 px); these are the split-screen surfaces to replace with `egui::Window`. `mode_bar::render_mode_bar` loops all 18 `ALL_SECONDARY_TOOLS` without consulting `AppConfig.layout` or `UserLevel`, so hidden/legacy entries still appear. `MAIN_TABS` exposes only Listen/Planes/Meteor even though `AppTab::Satellites` and `satellite_tab.rs` are fully implemented; satellite is only reached by pending decode. `radio_ui::sdrpp_module_inventory` duplicates tool entry points from the Tools menu. Core layout splits (`sdr_modules`, `radio_spectrum_rail`, ADS-B aircraft list/map, satellite pipeline/map) are operational and should stay.
- Follow-up agents must read this file, log their harness/model here, and complete the first unfinished checkbox before taking later work.
- The repository contains many historical task/report markdown files; prefer current source under `ez-gui`, `ez-proto`, and related crates over stale reports.
- Decision: keep Radio, ADS-B, Meteor, core receiver panels, Bookmarks, Scanner, Recorder, Scheduler, Settings, How To, Advanced, and Customize. Remove Discord and duplicate/integration menu entries from the primary Tools menu. AI remains available as a compact popup for explicit requests and contextual actions.
- Verification: `cargo check -p ez-gui --no-default-features`, focused layout tests, full `cargo test -p ez-gui --no-default-features` (686 passed, 4 ignored), and ignored radio visual render all passed. Existing unrelated warnings remain in audio/test helper code.
- Regression coverage: `utility_tools_render_as_floating_windows` exercises simultaneous How To and AI utility windows.
