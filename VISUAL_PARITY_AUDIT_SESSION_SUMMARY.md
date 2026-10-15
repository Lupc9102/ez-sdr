# Visual parity audit session summary

## Scope

Audited the remaining SDR++ inventory and spacing parity item after checking for Longcat/Claude swarm workers. This pass preserved the intentionally dirty worktree and made no product code changes.

## Evidence reviewed

- No `longcat`, `claude`, or `agy` worker process is active.
- Installed SDR++ capture: `/tmp/sdrpp-reference-current.png` (1920×1080).
- Current EZ-SDR Radio render: `/tmp/ez-sdr-previews/radio-1920x1012.png`.
- Installed SDR++ profile: `/home/lupc/.config/sdrpp/config.json`.

## Findings

- Geometry aligns on the 300 px sidebar and compact 40 px transport; EZ-SDR also has a 64 px Zoom/Max/Min rail.
- SDR++ opens modules individually in this order: Source, Radio, Recorder, Sinks, Frequency Manager, VFO Color, Band Plan, Display, Theme, Module Manager, Rigctl Server.
- EZ-SDR currently presents Source, Radio, Audio, and Display, followed by a collapsed aggregate module inventory.
- SDR++ stores `fftSize: 65536`; EZ-SDR exposes that option but starts at 2048 bins.
- Explicit SDR++ controls still absent are `fftHoldSpeed`, `fftSmoothing` enable/speed, and `fastFFT`.
- Native RF and audio behavior could not be validated in this environment.

## Changes

- Added the audit evidence and scoped remaining gaps to `UI_BUILD_TASK.md`.
- Left the literal parity task open because this audit does not prove native hardware/audio behavior or implement the remaining control inventory.

Harness/model: native Codex tools / GPT-6.
