# EZ-SDR Continuation Plan

## Purpose

This is the durable handoff document for the next agent. It will be updated throughout the overhaul so another agent can resume from verified facts rather than reconstructing the repository history.

## Product Goal

EZ-SDR should present one beginner-friendly application for four primary jobs:

1. Listen to live radio with spectrum and waterfall controls comparable to the core SDR++ experience.
2. Receive and track ADS-B aircraft without requiring a separate dump1090 installation or workflow.
3. Track weather satellites and decode supported downlinks into understandable products, growing toward the useful SatDump workflow subset.
4. Record, inspect, and replay IQ/audio data with clear device, signal, and processing status.

The interface should begin with the user's goal, select technically sound defaults, expose advanced controls progressively, and explain failures in plain language.

## Current Session

- Agent: root agent
- Harness/model: Codex CLI / GPT-6 Astra (continued from GPT-6)
- Started: 2026-09-17
- Live tracker: `EZ_SDR_OVERHAUL_TASK.md`
- Session summary: `EZ_SDR_SESSION_SUMMARY.md`

## Repository Safety

The task began with many modified and untracked files. They predate this session and may contain valuable unfinished work. Do not use destructive Git commands, broad formatting passes, or wholesale rewrites. Review diffs before editing a dirty file and keep changes narrow enough to distinguish from inherited work.

## Verified State

The README was rewritten around the verified desktop, daemon, browser, protocol, ADS-B, and LRPT architecture. It now uses package-specific commands and explicitly distinguishes compile-time backend support from physical-device validation.

The workspace currently has five Rust crates plus the web frontend:

- `ez-gui`: the legacy desktop application, with its own source ownership, audio DSP, ADS-B state, satellite UI, and embedded web remote.
- `ez-daemon`: a newer headless source owner, channelizer, DSP pipeline host, TCP protocol server, and HTTP/WebSocket server.
- `ez-web`: the daemon's TypeScript browser client.
- `ez-proto`: the shared Rust TCP message and codec crate.
- `dump1090`: the Mode-S/ADS-B decoder library and standalone binary.
- `lrpt-decode`: the Meteor LRPT QPSK/CCSDS/image library.

The two application architectures are still partly joined. Desktop daemon mode now subscribes to spectrum plus task-driven audio, ADS-B, and LRPT streams; audio, aircraft, and validated LRPT images reach their existing UI/output paths. The web client covers spectrum, audio, recording, aircraft, and LRPT image telemetry.

The web LRPT gap was closed during this continuation: `ez-web/src/ui/telemetry-panel.ts` renders daemon telemetry frames and per-APID grayscale images, and `ez-web/src/workflows/lrpt.ts` supplies a guarded beginner action for the supported Meteor-M2-3 preset.

### Baseline Validation

- Final `cargo test --workspace --no-default-features`: passed after the control-flood, recording, backpressure, and failure-path fixes. Current broad counts include 159 daemon library tests, 499 desktop library tests, 240 dump1090 library tests, 68 LRPT tests, five protocol tests, integration tests, and doc tests.
- `npm test` in `ez-web`: passed.
- `npm run build` in `ez-web`: passed.

### Prioritized Findings

#### P0: Broken User Paths

1. Fixed: web ADS-B startup now requests 2.4 MSPS and 1090 MHz before creating the decoder channel.
2. Fixed: desktop source workers use typed `SourceMessage` variants instead of magic IQ bytes.
3. Fixed: Demo and RTL-SDR are separate source modes even in hardware builds.
4. Fixed: compiled RTL-SDR, HackRF, and SoapySDR backends are exposed by daemon CLI source choices.

#### P1: Beginner and Integration Failures

1. Fixed: Quick Start advertises only executable desktop sources and applies presets to live source fields while preserving source lifecycle.
2. Fixed: spectrum-only operation no longer blocks rate changes, and the channelizer wideband rate is updated with the hardware request.
3. Fixed: desktop daemon mode integrates audio, aircraft, telemetry, and daemon-owned IQ recording controls/status.
4. Fixed: desktop LRPT CF32 decoding streams the recording through `decode_cf32_file`.
5. Fixed: the web client subscribes to LRPT telemetry and renders lock state, RS counts, and images.

#### P2: Product and Maintenance Gaps

1. Fixed: README claims and commands now match the current architecture and limitations.
2. Hardware and DSP implementations are duplicated across desktop, daemon, and dump1090 paths.
3. The repository root contains many stale historical reports. They are not reliable as current-state evidence and should eventually be archived after the active task is complete.

### Current Implementation Slice

The first repair slice is intentionally hardware-independent and reviewable:

1. Replace desktop source magic bytes with a typed worker message.
2. Split simulated and RTL-SDR hardware modes and make Quick Start advertise only executable choices.
3. Make Quick Start apply presets to live source fields.
   - Completed, including preserving the pre-wizard running/stopped lifecycle rather than implicitly starting hardware.
4. Repair the daemon/web ADS-B start path, including coordinated 1090 MHz and 2.4 MSPS setup. Implemented in `ez-web/src/workflows/adsb.ts` and verified by `ez-web/tests/adsb-workflow.test.ts` plus the production web build.
5. Make empty-channel sample-rate changes update the daemon channelizer's wideband rate. Implemented while allowing spectrum-only channels to remain active; verified by focused daemon state and channelizer tests.
6. Expose compiled hardware backends through the daemon CLI. Completed and compile-checked for `rtlsdr`, `hackrf`, and `soapy`; physical hardware remains unvalidated.
7. Stream LRPT recording reads instead of loading the entire file. Completed through `lrpt_decode::decode_cf32_file`; the desktop no longer calls `std::fs::read` for pass decoding.
8. Rewrite README around verified architecture and limitations. Completed; future edits should preserve the supported-versus-unimplemented distinction.

## Work Plan

### Phase 1: Establish Facts

- Read current repository guidance and historical plans.
- Map crate boundaries, entry points, feature flags, and frontend/daemon data paths.
- Inspect the inherited working-tree diff.
- Run focused compile, test, and web checks to identify current failures.

### Phase 2: Repair Core Workflows

- Restore build and protocol correctness first.
- Verify source discovery/opening, stream lifecycle, tuning, demodulated audio, spectrum transport, and shutdown behavior.
- Verify ADS-B sample flow through demodulation, tracking, protocol transport, and both UIs.
- Verify LRPT input, synchronization, packet/image assembly, and product presentation.

### Phase 3: Beginner Experience

- Replace feature-first navigation with clear task entry points where the existing architecture permits.
- Add useful no-device, disconnected, no-signal, and decoder-empty states.
- Surface safe presets and explain required sample rates, frequencies, gains, and antenna expectations.
- Keep expert controls accessible without making them prerequisites.

### Phase 4: Documentation and Validation

- Rewrite README claims around verified behavior and supported workflows.
- Run focused tests after each repair and broader checks at the end.
- Record hardware-dependent validation that remains outstanding.
- Leave exact commands, files, blockers, and next implementation slices for the next agent.

## Next Exact Steps

1. Validate RTL-SDR/HackRF/Soapy operation with physical devices; compilation and synthetic/replay testing are not substitutes.
2. Reduce duplicate hardware/DSP ownership across desktop, daemon, and dump1090 without regressing the now-working workflows.
3. Resolve the dump1090-derived-code licensing question before distribution.
4. Before a future handoff, re-run validation after any new edits. Do not run a broad formatter:
   inherited files contain unrelated formatting drift.

## Validation Evidence (2026-09-18)

- Final `cargo test --workspace --no-default-features`: passed, including 159 daemon library
  tests, 499 desktop library tests, 240 dump1090 library tests, 68 LRPT tests, five protocol
  tests, integration tests, and doc tests.
- `cargo test -p ez-daemon --lib --no-default-features`: 155 passed with loopback permission.
- `cargo test -p dump1090 --lib --no-default-features`: 240 passed with loopback permission.
- `cargo test -p lrpt-decode --lib --no-default-features`: 67 passed in the broad run; the
  later split-boundary regression test also passes (68 current tests).
- `cargo test -p ez-proto --lib --no-default-features`: 5 passed.
- `cargo test -p ez-gui quick_start --no-default-features`: 2 passed.
- `cargo check -p ez-gui --features rtlsdr`: passed.
- `cargo check -p ez-daemon` with no features and separately with `rtlsdr`, `hackrf`, and
  `soapy`: passed; HackRF/Soapy emitted missing pkg-config warnings, so hardware was not linked
  or exercised.
- `npm test` and `npm run build` in `ez-web`: passed after the ADS-B and LRPT workflow/telemetry changes (three test files).
- The final scoped `git diff --check` over all files touched in the continuation slice passes.
- Hardware control is now acknowledged through a bounded ingest request/reply queue; focused
  ingest and state tests pass. This can wait behind one blocking `read_iq` call and times out
  after two seconds rather than falsely reporting success. TCP/WS async workers now use
  `spawn_blocking`, and WS command intake remains responsive while hardware work is queued.
- A rejecting mock source verifies frequency/rate/gain errors preserve status, and TCP plus
  REST failure-path tests verify the source rejection reaches the client. Channelizer sample-rate
  metadata is mutated only after the physical source accepts the new rate.
- A root `git diff --check` still reports pre-existing whitespace in inherited dirty files
  (`ez-daemon/src/ingest.rs`, `ez-gui/src/source_manager.rs`, `lrpt-decode/src/ccsds.rs`,
  `lrpt-decode/src/image_builder.rs`, and `ez-web/tests/wire.test.ts`). The files changed in the
  final continuation slice pass a scoped diff check.

## Handoff Checklist

- Read `EZ_SDR_OVERHAUL_TASK.md` first and log your harness/model in its Tips section.
- Read this file completely, then inspect the latest Git status and diff before editing.
- Begin at the first unchecked tracker item unless a newer blocker is explicitly recorded.
- Mark completed items immediately with your harness/model identifier.
- Preserve inherited edits and update this plan with every verified architectural or behavioral fact that would save the next agent time.
