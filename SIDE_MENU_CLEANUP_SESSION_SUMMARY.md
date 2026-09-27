# Side Menu Cleanup Session Summary

## Target

Reduce the number of secondary side menus in EZ-SDR, remove low-value or duplicate entries, and make retained utilities open as compact floating windows so the main receiver workspace is not resized.

## Decisions

- Preserved the core Radio, ADS-B, and Meteor workspaces and their operational panels.
- Preserved Bookmarks, Scanner, Recorder, Scheduler, Settings, How To, Advanced, and Customize as utility tools.
- Kept the AI assistant as an explicit utility popup because it is useful for contextual radio questions.
- Removed Discord, MQTT, Web Remote, Rigctl, Band Plan, Frequency Manager, VFO Color, Theme, Layout, Module Manager, and duplicate SDR++ module shortcuts from the primary menu. Their backend/config compatibility remains where the application still needs it.
- Removed the ADS-B instruction banner from the main tracking layout; setup guidance remains available in How To.

## Changes

- Replaced the full-width `secondary_panel` and `ai_ask_panel` side drawers with resizable `egui::Window` popups sized for utility work.
- Added popup close behavior through the native window close control and retained the existing tool content handlers.
- Refactored How To to render its topic list and article inside the popup without nested root panels.
- Removed the sidebar's duplicate SDR++ module inventory and its deferred tool request plumbing.
- Updated the default layout and config test to match the curated tool set.
- Updated stale drawer comments and utility labels.

## Verification

- `cargo check -p ez-gui --no-default-features` passed.
- `cargo test -p ez-gui --no-default-features` passed: 686 passed, 4 ignored (plus all integration tests).
- Focused default-layout test passed.
- Ignored radio visual render passed at 1920×1012, 1400×900, and 1000×700.
- `git diff --check` passed.
- Added a regression test proving the How To utility and AI assistant render while remaining active as floating windows.

The build still reports pre-existing warnings in `audio_output.rs` and visual test helpers. Generated signal/tile artifacts were kept outside the implementation diff or restored where tracked.

## Harness

- Primary implementation and verification: Codex / GPT-6.
- UI inventory: Codex / GPT-6 (`ui_map` subagent).
- Keep/remove audit: Codex / GPT-6 (`ux_decision` subagent).
