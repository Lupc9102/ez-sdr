# Ship runtime audit

## Scope and identity

Native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider/model routing is not exposed. Built-in session agents only. This bounded review covered source startup/stop/restart/EOF, playback queue handling, recording lifecycle/save failures, and daemon connection/control shutdown. Existing dirty work was preserved. Root owns app/config/UI integration.

## Changes

- `ez-gui/src/daemon_client.rs`: connecting, waiting for Welcome, and blocked command writes now observe shutdown without waiting for the full five-second operation timeout. Outgoing command failures retain a visible error. During connected shutdown, recording-stop commands queued by `SourceManager::stop()` get a bounded grace period before Detach; otherwise an intentional source stop could leave a remote recording running because daemon recordings survive disconnect. Added pending-operation cancellation and two real loopback regressions.
- `ez-gui/src/recorder_panel.rs`: WAV headers use the actual audio sample rate. `set_audio_sample_rate` finalizes/stops an active WAV session on a rate change and leaves a visible message, preserving finalization errors. Root wired the setter at app logic entry, successful audio startup, and recorder panel rendering. Repeated filename templates select unused suffixes; create-new file opens prevent overwrites even if another writer races. Metadata uses JSON serialization, so quoted filenames remain valid. Missing formats, invalid filenames, directory failures, metadata-write failures, and WAV-write failures surface explicitly; failed WAV writes stop the session and do not count its unwritten tail. Drop joins the IQ writer and finalizes WAV so queued recording data is flushed before returning.
- `ez-gui/src/source_manager.rs`: extreme and nonfinite CF32 replay samples no longer overflow integer conversion and terminate the worker. Finite out-of-range components clamp safely and invalid components become neutral IQ bytes.
- `ez-daemon/src/recording.rs`: recording files use create-new semantics; final flush errors are retained and returned by stop rather than reporting a successful recording. Existing writer errors also propagate through stop.
- `ez-daemon/src/ingest.rs`: deliberate shutdown now publishes disconnected hardware status, preserving any recorded source error.

No production changes were needed in `audio_output.rs` or `audio_resampler.rs`. Their existing device-selection, failure-state, nonblocking callback, bounded backlog, stereo-frame alignment, gain, and streaming rate-conversion behavior was reviewed and tested.

## Verification

Exact focused commands and logs:

| Command | Result | Log |
| --- | --- | --- |
| `cargo test -p ez-gui --no-default-features --offline --lib source_manager::tests -- --nocapture` | 44 passed | `/tmp/ez-sdr-runtime-source-tests.log` |
| `cargo test -p ez-daemon --offline --lib recording::tests -- --nocapture` | 10 passed | `/tmp/ez-sdr-runtime-daemon-recording-tests.log` |
| `cargo test -p ez-daemon --offline --lib ingest::tests -- --nocapture` | 5 passed | `/tmp/ez-sdr-runtime-ingest-tests.log` |
| `cargo test -p ez-gui --no-default-features --offline --lib daemon_client::tests::shutdown_cancels_a_pending_network_operation_without_waiting_for_its_timeout -- --nocapture` | 1 passed in 0.10 s | `/tmp/ez-sdr-runtime-cancellation-tests.log` |
| `cargo test -p ez-gui --offline --lib audio_output::tests -- --nocapture` | 12 passed | `/tmp/ez-sdr-runtime-audio-output-tests.log` |
| `cargo test -p ez-gui --offline --lib recorder_panel::tests -- --nocapture` | 26 passed | `/tmp/ez-sdr-runtime-recorder-tests.log` |
| `cargo test -p ez-gui --offline --lib audio_resampler::tests -- --nocapture` | 3 passed | `/tmp/ez-sdr-runtime-audio-resampler-tests.log` |

The new recorder cases cover static-template collisions and quoted JSON, actual 44.1 kHz WAV duration and a transition to 48 kHz, invalid formats/paths, write failures, and all queued IQ bytes flushed on Drop. The CF32 regression exercises NaN, both infinities, extreme finite values, normal values, and clean EOF. The daemon flush regression uses a read-only temporary file handle to produce a real I/O error without hardware or privileged paths.

```sh
rustfmt --edition 2021 --check ez-gui/src/source_manager.rs ez-gui/src/recorder_panel.rs ez-gui/src/daemon_client.rs ez-daemon/src/recording.rs ez-daemon/src/ingest.rs
git diff --check -- ez-gui/src/source_manager.rs ez-gui/src/recorder_panel.rs ez-gui/src/daemon_client.rs ez-daemon/src/recording.rs ez-daemon/src/ingest.rs
```

Both formatting/patch checks pass.

## Execution limits

The real loopback test `daemon_client::tests::drop_cancels_a_silent_handshake_promptly` could not bind `127.0.0.1` in the network sandbox (EPERM). An escalation request was not executed: automatic approval review failed HTTP 403 because its reviewer requested unprefixed `gpt-5.6-luna` while the provider permits only `codex/...` routes. This was an approval-infrastructure failure, not a determination that the action was unsafe. Parent was notified; no bypass or repeated escalation was attempted. Both new loopback tests compile, and the socket-free cancellation test passes, but full network shutdown behavior still requires a permitted loopback run. Initial blocked-test output: `/tmp/ez-sdr-runtime-handshake-before.log`.

No physical SDR device, native window, real audio device, speaker callback scheduling, or live external daemon was exercised. Local replay/file I/O, in-memory DSP/audio callbacks, and daemon ingest/recording regressions are the verified scope.
