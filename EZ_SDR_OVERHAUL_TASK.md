# EZ-SDR Unified Overhaul Task

## Target

Transform EZ-SDR from a collection of partially integrated SDR features into a coherent, beginner-friendly application that can eventually replace the common workflows served by SDR++, dump1090, and SatDump. The work must be evidence-driven: inspect the actual desktop, daemon, web, protocol, ADS-B, and LRPT implementations; preserve existing uncommitted work; repair concrete build, runtime, integration, and usability failures; simplify the first-run path; and document remaining gaps honestly.

The intended product experience is one application with task-oriented entry points such as listening to radio, tracking aircraft, receiving a satellite image, and inspecting spectrum. A beginner should be able to connect hardware, choose a goal, receive safe defaults and clear status feedback, and recover from missing hardware or configuration without understanding SDR terminology first.

## Tasklist

- [x] Initialize the task tracker and record agent identity — Codex CLI / GPT-6 (root agent)
- [x] Read the current README and inventory the repository at a high level — Codex CLI / GPT-6 (root agent)
- [x] Inspect repository guidance, existing plans, dirty changes, and current architecture without overwriting prior work — Codex CLI / GPT-6 (root agent)
- [x] Delegate whole-project structure analysis through the required `agy` protocol and incorporate useful findings — agy harness / Gemini 3.6 Flash high reasoning, coordinated by Codex CLI / GPT-6
- [x] Establish an honest baseline with targeted Rust and web checks — Codex CLI / GPT-6 (root agent)
- [x] Audit the beginner journey across desktop and web clients — Codex CLI / GPT-6 (root agent)
- [x] Audit SDR++, dump1090, and SatDump replacement claims against implemented workflows — Codex CLI / GPT-6 (root agent)
- [x] Prioritize failures by user impact, implementation risk, and testability — Codex CLI / GPT-6 (root agent)
- [x] Fix critical build, protocol, runtime, and data-flow defects — Codex CLI / GPT-6 Astra (root agent)
- [x] Coordinate web ADS-B startup at 1090 MHz / 2.4 MSPS and keep daemon channelizer sample-rate state synchronized — Codex CLI / GPT-6 Astra (root agent)
- [x] Expose compiled RTL-SDR, HackRF, and SoapySDR daemon sources through CLI configuration — Codex CLI / GPT-6 Astra (root agent)
- [x] Stream desktop LRPT CF32 decoding instead of loading complete pass recordings into memory — Codex CLI / GPT-6 Astra (root agent)
- [x] Improve first-run guidance, empty states, status/error messages, and task-oriented navigation — Codex CLI / GPT-6 Astra (root agent)
- [x] Render daemon LRPT lock telemetry and decoded grayscale images in the web client — Codex CLI / GPT-6 Astra (root agent)
- [x] Add a beginner-oriented Meteor-M2-3 start action to the web LRPT panel — Codex CLI / GPT-6 Astra (root agent)
- [x] Keep Quick Start source lifecycle consistent instead of always starting after applying a preset — Codex CLI / GPT-6 Astra (root agent)
- [x] Subscribe desktop daemon mode to task-driven audio, aircraft, and LRPT streams — Codex CLI / GPT-6 Astra (root agent)
- [x] Render live daemon LRPT telemetry and images in the desktop satellite decoder — Codex CLI / GPT-6 Astra (root agent)
- [x] Route desktop recording controls and live status through daemon-owned recording in Daemon mode — Codex CLI / GPT-6 Astra (root agent)
- [x] Acknowledge daemon hardware commands so REST/TCP callers receive backend failures — Codex CLI / GPT-6 Astra (root agent)
- [x] Prove hardware backend rejections propagate through ingest, TCP, and REST — Codex CLI / GPT-6 Astra (root agent)
- [x] Keep web control and data planes responsive during acknowledged hardware-command floods — Codex CLI / GPT-6 Astra (root agent)
- [x] Align README claims and setup instructions with the code that actually ships — Codex CLI / GPT-6 Astra (root agent)
- [x] Rewrite the README around verified desktop/daemon/browser workflows and explicit limitations — Codex CLI / GPT-6 Astra (root agent)
- [x] Add or update meaningful tests for repaired behavior — Codex CLI / GPT-6 Astra (root agent)
- [x] Run focused validation, then workspace-wide checks where practical — Codex CLI / GPT-6 Astra (root agent)
- [x] Produce a detailed continuation plan with architecture, completed work, open risks, and exact next steps — Codex CLI / GPT-6 Astra (root agent)
- [x] Produce the required standalone session summary — Codex CLI / GPT-6 Astra (root agent)

## Tips

- Active agent: root agent using the Codex CLI harness and GPT-6 model; picked up the task on 2026-09-17.
- Continued by the root agent using the Codex CLI harness and GPT-6 Astra model on 2026-09-18; resumed at the web ADS-B and daemon sample-rate coordination slice.
- Resumed from checkpoint by the root agent using the Codex CLI harness and GPT-6 Astra model on 2026-09-18; first action was to poll the existing workspace test session and preserve its failure evidence.
- Architecture audit delegate: `agy` harness using Gemini 3.6 Flash with high reasoning; completed a read-only audit on 2026-09-17. Its report is captured in `EZ_SDR_CONTINUATION_PLAN.md` and was verified against source before prioritization.
- The working tree was already heavily modified before this task. Preserve and inspect those changes; do not reset, clean, or overwrite them.
- The repository already contains many historical audits and handoff files. Treat them as leads, verify every claim against current code, and keep this tracker as the live source of truth for this session.
- The README describes an older desktop-centric layout and appears inconsistent with the current daemon/web architecture. Verify commands and features before repeating them.
- Update each checkbox immediately after its work is complete and append the completing harness/model identity.
- Baseline validation: `cargo test --workspace --no-default-features` passes when allowed to open loopback sockets; `npm test` and `npm run build` pass in `ez-web`.
- Web ADS-B start now has a dedicated workflow that requests 2.4 MSPS and 1090 MHz before creating the decoder; daemon sample-rate changes now update channelizer geometry while allowing the always-on spectrum pipeline to remain active. Verified with three focused daemon state tests, 23 channelizer tests, two web test files, and a production web build.
- Daemon `--source` now advertises `rtlsdr`, `hackrf`, or `soapy` only when the corresponding feature is compiled; `--device` selects an RTL-SDR identity or Soapy argument string. All three feature-specific `cargo check` runs pass; the environment reports missing pkg-config metadata for HackRF/Soapy, so no physical-device claim is made.
- Desktop LRPT now calls `lrpt_decode::decode_cf32_file`, which streams buffered CF32 data and preserves 8-byte sample alignment across arbitrary read boundaries. `lrpt-decode`'s focused file test and `cargo check -p ez-gui --features rtlsdr` pass.
- A three-byte-at-a-time reader regression test now proves CF32 streaming preserves complete 8-byte complex samples across misaligned reads.
- README now documents the current six-part architecture, correct package-specific run commands, compile-time hardware backends, beginner workflows, and the gaps versus SDR++, dump1090, and SatDump without claiming physical-device validation.
- Web channels of kind `LrptTelemetry` now attach to their binary stream and render Costas/frame lock, RS counts, and per-APID grayscale canvases. Web tests pass and the production build succeeds.
- The empty web LRPT panel now offers `Start Meteor LRPT`, tunes the supported M2-3 preset, validates capture bandwidth, refuses to disrupt other active decoder/audio channels, and creates the telemetry channel. Three workflow tests pass.
- Quick Start now preserves whether a source was stopped or running: presets update the live source fields in both cases, but only restart a source that was already active. The completion copy now truthfully tells stopped users to press Start. Focused Quick Start tests and the RTL-SDR GUI check pass.
- Desktop daemon mode now maintains stable task channels for audio, ADS-B, and Meteor LRPT in addition to spectrum, routes daemon audio to playback, maps aircraft snapshots into the existing panel, renders validated per-APID LRPT images and lock/RS status in the decoder panel, and tears down inactive task pipelines. The RTL-SDR GUI check, 13 source-manager tests, and seven decoder-panel tests pass.
- Desktop Daemon mode now records the daemon's actual wideband IQ instead of opening an empty local file. Start/stop commands use channel 1, periodic and final `RecordingStatus` updates show daemon path, bytes, and duration, and the GUI client prioritizes control events over bounded high-rate stream traffic. The focused end-to-end recording test, 21 recorder tests, and RTL-SDR GUI check pass.
- Daemon frequency, sample-rate, and gain commands now use bounded request/reply messages to the sole hardware-owning ingest thread. REST and TCP report backend rejection/timeout instead of claiming success as soon as a command is queued. Four ingest tests and the channelizer retune test pass.
- A rejecting mock `IqSource` now proves frequency/rate/gain errors preserve hardware status, while focused TCP and REST tests prove the backend error text reaches callers. Sample-rate geometry is updated only after the hardware accepts the new rate.
- Web control intake now answers Ping immediately, queues state-changing commands separately, coalesces adjacent hardware slider updates, and executes potentially blocking hardware acknowledgements on Tokio's blocking pool. The previously failing control-flood resilience test and seven focused WebSocket tests pass.
- Final validation after repairing the control-flood regression and desktop daemon recording: `cargo test --workspace --no-default-features` passes, including 159 daemon library tests, 499 desktop library tests, 240 dump1090 library tests, 68 LRPT tests, protocol tests, integration tests, and doc tests. `npm test` (three files) and `npm run build` also pass in `ez-web`; the scoped diff check is clean.
