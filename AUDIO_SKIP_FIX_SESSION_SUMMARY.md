# Audio Skip Fix Session Summary

## Target

Remove the barely audible periodic skips reported during live RTL-SDR playback
while preserving the existing RTL-SDR, spectrum, ADS-B, and Meteor behavior.

## Completed

- Traced the skip to a timing mismatch: RTL-SDR supplies approximately 8 ms
  blocks, while the GUI previously woke every 33 ms and drained only four blocks.
  The resulting one millisecond deficit per tick eventually filled the source
  queue and dropped IQ blocks.
- Increased the CPAL audio worker queue to 32 audio batches.
- Increased the demodulator queue to 16 batches.
- Drained up to eight source blocks per active UI cycle.
- Changed the active receiver repaint interval to 16 ms so the UI keeps pace with
  the live sample clock and has room to absorb scheduler jitter.
- Preserved all existing unrelated worktree changes.
- Restarted the release GUI on display `:1`; current process PID is `303762`.

## Verification

- `cargo fmt --all -- --check` passed.
- `cargo check -p ez-gui --offline` passed.
- `cargo check -p ez-gui --no-default-features --offline` passed.
- Audio output tests: 12 passed.
- Demodulator tests: 67 passed.
- `cargo build -p ez-gui --release --features rtlsdr --offline` passed.
- Release GUI is running for sustained playback testing.

## Agent

- Codex / gpt-6, 2026-09-26.
