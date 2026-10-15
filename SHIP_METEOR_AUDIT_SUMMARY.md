# Offline Meteor ship audit

Harness/model: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider ID unavailable. Native session collaboration only; no Morph or agy.

## Confirmed failures and fixes

1. **Large input blocks lost valid CADUs.** Six continuously encoded synthetic CADUs produced 24 successful RS codewords with small chunks but only 4 with one large block. Frame synchronization now searches before trimming retained noise, and the decoder processes every valid frame from its selected hypothesis. The same fixture now produces 24 successful codewords in both cases.
2. **Progress backpressure could stall decoding and retain large image copies.** A full bounded progress channel reproduced a blocked decoder. Progress now uses nonblocking sends, skips snapshot creation when the queue is full or already contains two updates, and directly samples grayscale previews with each side at most 1024 pixels. The GUI worker uses a one-slot internal progress channel. Final images retain full resolution.
3. **Long channel previews could panic at GPU texture limits.** A 1568×4096 image reproduced an egui texture-size panic against a 1024-side limit. Display copies now respect the active GPU limit and a 2048-side cap, while preserving the original aspect ratio in the scroll view. PNG export uses the original pixels; the regression verifies every exported pixel.
4. **Nonfinite CF32 samples produced a misleading no-sync failure.** NaN input previously consumed the recording and ended as a no-CADU-sync error. The file worker now rejects nonfinite float components before feeding the affected block to the decoder and reports the zero-based complex sample index. Existing empty-file, sample-alignment, rate and regular-file checks remain in place.

## Changed sources

- `lrpt-decode/src/lib.rs`: complete frame processing, nonblocking bounded progress, continuous multi-CADU test fixture and regressions.
- `lrpt-decode/src/frame_sync.rs`: parse incoming bits before retention trimming.
- `lrpt-decode/src/image_builder.rs`: bounded previews sampled directly from stored pixels and a full-resolution preservation regression.
- `ez-gui/src/decoding_panel.rs`: bounded progress channel, CF32 validation, GPU-safe display copies and export/UI regressions.

Prior offline-only behavior in `quick_start.rs`, `satellite_panel.rs` and `satellite_tab.rs` remains intact. Ordinary Radio recording and root-owned source/configuration integration are outside this audit's edits.

## Final verification

- `cargo test -p lrpt-decode --offline --lib`: **71 passed**, 0 failed, 7.37 seconds.
- `cargo test -p ez-gui --offline --no-default-features decoding_panel::tests --lib`: **22 passed**, 0 failed, 1.22 seconds. Includes synthetic signed-CS8 import→image recovery→PNG export, cancellation, background dialogs, malformed input, progressive rendering and constrained GPU textures.
- `cargo check -p ez-gui --offline --features 'audio rtlsdr'`: passed.
- Rustfmt on the four affected sources and scoped `git diff --check`: passed.

## Integration and practical limits

`DecodeProgress.preview` now consistently supplies bounded, downsampled images, as its API documentation permits. Daemon consumers also receive these smaller progress previews. `DecodeResult.images` and final PNG exports remain full resolution.

Validation used synthetic IQ fixtures and headless egui frames, including an imposed texture-size limit. It did not exercise a native GPU, real SDR hardware, off-air Meteor captures or real user configuration. These results establish regression coverage for the reproduced failures, not off-air acceptance.
