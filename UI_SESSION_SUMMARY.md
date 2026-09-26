> Follow-up implementation and current verification: [RADIO_PARITY_SESSION_SUMMARY.md](RADIO_PARITY_SESSION_SUMMARY.md). This document preserves the earlier rebuild report.

# EZ-SDR UI rebuild checkpoint — 2026-09-23

## Continuation — 2026-09-25 (Codex / GPT-6)

Added a native loopback-only Hamlib/rigctld-compatible server in `ez-gui/src/rigctl.rs`. The server supports frequency (`f`/`F`), mode and passband (`m`/`M`), AF volume (`v`/`V`), and quit (`q`) commands. Network requests are queued to `CentralApp` and applied through the existing shared tuning, demodulation, and volume paths. Configuration now persists `rigctl_enabled` and `rigctl_port`, and the SDR++ Modules inventory opens a dedicated Rigctl drawer with live status.

Verification for this continuation: `cargo check --workspace --all-features --offline` passed; the no-default-feature GUI library suite passed **657 tests** with 3 ignored (the visual render test plus two rigctl socket tests); the two rigctl socket tests passed with loopback permission; and formatting was applied with `cargo fmt --all`. Socket tests are ignored in restricted runs because the environment can deny loopback binds.

## Continuation — 2026-09-24 (Codex harness / GPT-6)

The Radio workspace was compared against the installed SDR++ 1.3.0 window at its native 1920×1012 viewport. The Radio render now keeps the verified 300 px sidebar and 300 px FFT pane, starts the plot directly below the transport bar, exposes the SDR++-style vertical Zoom/Max/Min rail, uses radio selectors for the demod modes, and keeps spectrum controls in the Display section so the plot is not pushed down by a duplicate menu row. The top shell continues to expose Radio, ADS-B, and Meteor together.

The comparison used `/tmp/sdrpp-window.png` from the installed `/usr/bin/sdrpp` and the actual egui tessellation renders in `/tmp/ez-sdr-previews/radio-1920x1012.png`, `/tmp/ez-sdr-previews/radio-1400x900.png`, and `/tmp/ez-sdr-previews/radio-1000x700.png`. ADS-B and Meteor were rerendered at the same three viewport sizes; the map remains centered on the configured observer and Meteor remains import/decode-only.

The Radio sidebar now has a collapsed SDR++ Modules inventory for Recorder, Sinks, Frequency Manager, VFO Color, Band Plan, Theme, Module Manager, and Rigctl Server. Clicking an entry opens the nearest existing EZ-SDR tool drawer. Sinks now opens a dedicated drawer that reuses Radio's output-device, sample-rate, mute, and volume controls. The toolbar tuning control uses compact center/normal glyphs rather than a text-only toggle. The release build completed successfully, and the release binary stayed running for a 5-second Xvfb smoke launch (exit 124 from the intentional timeout; only expected ALSA device warnings were emitted).

The replay source now records file size and advances replay byte position as chunks are consumed, marking the position at EOF. This repairs the app-level replay lifecycle regression exposed by the full GUI test suite. Focused Radio tests pass 16/16. The full no-default-feature GUI library run passed **652 tests** (1 ignored); `cargo check --workspace --all-features --offline`, the release build, and `git diff --check` pass. The visual render test passes for all three workspaces and all nine viewport/workspace combinations.

The installed SDR++ plugin inventory is now exposed as a collapsed “SDR++ Modules” section in the Radio sidebar, with shortcuts into the equivalent EZ-SDR tools. Native window input, physical RF reception, and audible output remain unverified in this restricted environment. The 978 path uses the separately installed dump978-fa JSON feed and states that dependency in the UI; it is not an in-process Rust UAT demodulator.

On 2026-09-25, the Sinks route was verified in the current tree and all nine Radio/ADS-B/Meteor renders were regenerated. Band Plan now has its own drawer backed by the live spectrum overlay and ITU region selector. The Rigctl shortcut now opens the native loopback Hamlib/rigctld-compatible drawer. The full GUI library suite passed **657 tests** with 3 ignored (the opt-in render test and two socket tests in restricted mode). A prior restricted run hit a recording-status deadline once; the test now allows 10 seconds for its synthetic source and recorder worker under suite contention, and the loopback-enabled full run passed. `cargo check --workspace --all-features --offline`, `cargo fmt --check`, the release build, and a 4-second Xvfb release smoke launch passed (the intentional timeout returned 124; ALSA reported the expected missing virtual audio device).

The ADS-B map status was also refined: a usable map with a few failed tiles now reports “Some map tiles unavailable · retrying”; “Map unavailable” is reserved for the case where no tile has loaded. The updated 1090 map was rendered at all three review sizes. The Rigctl inventory tooltip identifies the loopback protocol server and its supported command set.

## Outcome

The new three-tab Rust UI is integrated and buildable. Radio was rebuilt instead of retaining the old cards; ADS-B now receives real decoded 978 UAT reports through dump978-fa; Meteor imports signed CS8 recordings and decodes them without recording controls. Under the requested feature-focused standard, the UI goal is complete; exact SDR++ source/plugin parity and native hardware acceptance remain separate validation work.

Work used built-in session subagents through the inherited model route. No Morph or agy was used after the user's steering. Explicit `gpt-6-astra` launches failed provider routing; the tool rejected `codex/gpt-6-astra` as an unknown override. The inherited route worked, but its exact provider identifier is not exposed, so the report does not assert it was Astra.

## Changes completed

- Replaced the Radio UI with a compact play/stop/menu/frequency/mute/volume bar and Source, Radio, Audio and Display sections. The installed SDR++ profile supplied the 300 px sidebar and 300 px FFT pane dimensions. Removed obsolete SdrPanel and Listen-header implementations while preserving shared demodulation/frequency helpers.
- Rebuilt navigation as Radio / ADS-B / Meteor, removed automatic demo startup and first-run gating, made utility tools accessible from Tools, and removed satellite recording navigation from Meteor.
- Connected frequency digits, direct entry, spectrum clicks, keyboard jumps, scanner and remote tuning to actual source retuning. Radio modes expose working local bandwidth, squelch, AGC, de-emphasis, high-pass and noise-blanker controls.
- Fixed receiver termination on queue saturation. Live capture drops an overflow block while continuing; replay retains sample/EOF ordering through cancellable backpressure. Stop wakes paced workers; unexpected disconnect becomes a visible error.
- Kept active source/daemon polling at 16 ms, corrected daemon volume scaling and device-rate conversion, surfaced audio startup failures and allowed stop/start recovery.
- Added bounded off-thread dump978-fa JSON TCP reception with reconnect/cancel, address-class handling, source ownership controls and map/table merge. 978 requires the separately installed external decoder; it is not Rust-native UAT demodulation.
- Corrected clockwise SVG aircraft headings. Hardened map downloads with size/dimension/time limits, off-thread decoding, bounded retry backoff, missing-tile status/grid and muted dark-theme tiles. OSM attribution remains visible.
- Isolated Meteor imported-file state from live satellite state. Added aligned streamed CS8/CF32 reads, rate validation, byte progress, cancellation, progressive/final channel images, asynchronous file dialogs and non-overwriting PNG export.
- Corrected Meteor's horizontal results-layout inheritance, duplicate status footer, invisible slider rails, incorrectly premultiplied band colors, FFT/waterfall overlap and narrow-window control clipping.
- Removed unused WGPU and broad image-codec default features. Desktop uses Glow; image handling includes PNG/JPEG. No extra crates were needed for the UAT receiver.
- Rewrote README around the actual desktop workflows, external UAT setup, offline Meteor imports and current build requirements. It no longer instructs users to build the removed ez-web frontend or record Meteor passes in the Meteor workflow.

The worktree contained extensive earlier changes. Those were preserved; the session does not claim authorship of all changes reported by `git diff`.

## Verification

- `cargo build -p ez-gui --release --offline`: passed; optimized desktop executable is 19,684,496 bytes (19.7 MB / 18.8 MiB) with speaker playback enabled. Run `target/release/ez-gui`. The default binary uses Demo/File/Daemon; local RTL-SDR needs a `--features rtlsdr` build.

- `cargo check --workspace --all-features --offline`: passed. Optional HackRF/Soapy pkg-config warnings remain; compilation is not device validation.
- Final component run: **507 GUI library tests, 4 GUI integration tests, 67 LRPT tests, 5 protocol tests, 244 dump1090 library and 244 binary tests passed**. Ten distinct network tests were explicitly excluded after root-context socket binding failed with `Operation not permitted`. The visual artifact test is separately opt-in.
- Focused suites include 8 Radio interaction/lifecycle tests, 20 Meteor tests, 20 source-manager tests, 21 ADS-B panel tests and 3 daemon-audio resampler tests.
- The UAT delegate separately reported 12 UAT tests passing, including actual loopback reconnect/cancellation. Root's restricted context cannot reproduce that socket test; the distinction is retained rather than reporting an unrestricted full-suite pass.
- Meteor evidence includes a documented synthetic signed CS8 OQPSK recording decoded through Viterbi/RS/CCSDS/MSU-MR to a 1568×8 image and exported PNG, checking every expected pixel. This is not an independently captured off-air pass.
- Rendered actual egui widgets at 1400×900 and 1000×700 with the default audio-enabled build using a CPU tessellation renderer. Examined empty workspaces and explicitly synthetic ADS-B/UAT aircraft fixtures; fixed defects and rerendered. Native window, GPU, audio device and real SDR operation are separate outstanding checks.
- `rustfmt --check` on touched Rust files and `git diff --check`: passed.
- The final optimized rebuild after the last spectrum/layout edits passed; executable timestamp is newer than the modified source.

## Visual evidence

These are actual widget renders, not image-generation mockups. Aircraft named TEST-EAST/TEST-NORTH are test fixtures, not live observations.

- [Radio](artifacts/ui-review/radio-1400x900.png)
- [1090 map with synthetic aircraft](artifacts/ui-review/adsb-fixture-1400x900.png)
- [978 map with synthetic aircraft through the UAT merge path](artifacts/ui-review/uat-fixture-1400x900.png)
- [Meteor at 1000×700](artifacts/ui-review/meteor-1000x700.png)

## Requirement audit and remaining work

| Requirement | Evidence / state |
|---|---|
| Native Rust desktop with three tabs | Implemented in eframe/egui; compilation and actual widget renders verified. Native window acceptance remains open. |
| Listen to ATC/radio | AM/NFM/WFM/USB/LSB and source/audio paths implemented and tested in software. No physical reception/audio acceptance in this session. |
| SDR++-style radio controls and spacing | Feature coverage is implemented with the verified compact geometry and module shortcuts. Exact source/plugin ordering and physical RF/audio acceptance remain unverified. |
| ADS-B map, 1090/978 picker, little plane SVGs | Implemented; software parser/lifecycle/map tests and rendered heading fixtures verified. 978 requires dump978-fa. Field reception remains unverified. |
| Meteor CS8 decoding only | Implemented, independently isolated, synthetic image-to-PNG and UI regressions pass. No recording controls in the Meteor workspace. |
| Snappy/lightweight/not eye tiring | Removed unused render/codec stacks, eliminated queue-termination and blocking-dialog defects, bounded workers and corrected contrast/layout. Release/native performance measurements remain open. |
| Replace prior UI | Old Radio UI implementations deleted; new shell/Radio controls installed. Shared DSP and supplementary tools retained. |

Remaining parity work is limited to exact SDR++ plugin ordering/spacing, a few reference-only display toggles, and native RF/audio acceptance on hardware. DSB/CW, stereo/RDS, independent RF bandwidth, device enumeration/refresh, IQ correction/inversion/decimation, tuning modes, tone squelch/IF noise reduction, audio-device selection, and the requested workspaces are implemented and covered by tests.

Automatic approval review prevented upstream source/browser reference access because its configured model route was rejected with 403. This was an approval infrastructure failure, not a safety rejection. No bypass was attempted. Native X display access was also unavailable in the root environment. Those constraints do not prove layout parity or runtime success.

## Related records

- UI_REBUILD_TASK.md — current task and agent identities
- SDRPP_UI_REFERENCE.md — installed reference and explicit parity gaps
- UAT_SESSION_SUMMARY.md / UAT_RECEIVER.md — 978 integration and setup
- METEOR_SESSION_SUMMARY.md — decoder tests and asynchronous UI
- INTEGRATION_REVIEW_SUMMARY.md — source-lifecycle findings and fixes

## Final verification continuation — 2026-09-26

Native Codex / GPT-6 corrected stale help copy that described CW, WFM stereo/RDS, Meteor import, and 978 UAT as unavailable. The final gates passed: 681 GUI library tests (4 ignored), GUI integration tests, 71 LRPT tests, 248 dump1090 tests, 5 protocol tests, 166 daemon tests (2 ignored), release radio throughput (2 passed), all-feature workspace checking, release build, all 12 software-rendered workspace variants, and a five-second Xvfb release smoke launch. See [UI_FINAL_SESSION_SUMMARY.md](UI_FINAL_SESSION_SUMMARY.md) for the complete final report.
