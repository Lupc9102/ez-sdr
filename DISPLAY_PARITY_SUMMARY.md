# Display parity session report

Native Codex session subagent `/root/radio_rebuild`, inherited GPT-6-family route; exact provider model identifier is not exposed. No Morph or agy. Edited only `ez-gui/src/spectrum.rs` and scoped display task/report documents.

## Implemented

- Validated power-of-two FFT sizes 256–65536. Invalid requests return false and preserve the live stream; reapplying an unchanged size does not reallocate/reset the display.
- Real streaming unsigned-byte IQ accumulation in a fixed-size circular window, including byte pairs split across calls. Short blocks wait for a complete real window; no zero padding or interpolation. Large blocks feed the whole stream. At most 4 transforms run per call; an oversized burst ends with the newest real window, intentionally skipping intermediate display windows.
- Configurable 1–120 FFT frames/s using sample-clock hop sizes, with overlapping FFT windows when the hop is shorter than the transform. Daemon frames use timestamp deltas to limit display cadence; malformed/nonfinite/unsupported-size frames are ignored before allocation. Backwards daemon timestamps reset the cadence.
- Correct local dBFS normalization and coherent window gain: full-scale bin-centered tone level is independent of FFT size/window. Previously raw ADC byte amplitude produced values about 42 dB above the advertised full-scale reference.
- Waterfall visibility and real-frame scrolling. UI repaint alone no longer appends repeated stale rows. Hiding the waterfall gives the spectrum the remaining height and skips waterfall collection/texture work. Pending rows are uploaded independently when several spectra arrive between draws.
- Waterfall width aggregated with peak preservation to at most 2048 columns and total retained RGBA storage limited to 8 MiB. Dimensions respect egui's reported texture limit. Full-resolution spectrum bins remain available for CSV export; PNG width matches the aggregated waterfall.
- Full-resolution plot traces/gradient/peak-hold meshes aggregate by screen columns (capped 4096) so a 65536-bin transform does not imply 65536 drawn segments per frame. Waterfall texture UV cropping now tracks zoom/pan labels.
- Display-only time-constant SNR smoothing. Peak-hold decay also follows elapsed spectrum time instead of assuming 60 UI frames/s. SNR smoothing does not alter detector peak/noise levels.
- USB/LSB VFO overlays use one-sided RF bandwidth. `vfo_signal_level()` returns the strongest FFT bin inside that RF passband, preventing unrelated channels or the wrong sideband from opening a display-based squelch gate.
- A public `fft_controls_enabled` flag (default true) disables all FFT size/window controls together in the spectrum Display menu when the daemon owns them. The disabled scope uses the sidebar's tooltip, "The daemon supplies its own FFT bins and window"; averaging and other display controls remain enabled. Root sets the flag from source mode before rendering.
- Corrected straight RGBA constants throughout the spectrum to use `Color32::from_rgba_unmultiplied`. This fixes the bright RF passband overlay and the same alpha bug in bookmark, band-plan, marker, and label colors without changing palette values or intended alpha. The two waterfall bookmark dimmers scale already-premultiplied RGB and alpha together with `gamma_multiply`, retaining their intended 90/130 alpha values.

## Integration APIs

The following APIs are available for root's radio/config/application integration:

```rust
SpectrumAnalyzer::valid_fft_size(size: usize) -> bool
set_fft_size(size: usize) -> bool
fft_size() -> usize
set_fft_rate(frames_per_second: u32)
fft_rate() -> u32
set_waterfall_visible(visible: bool)
waterfall_visible() -> bool
set_snr_smoothing(enabled: bool, time_constant_secs: f32)
snr_db() -> f32
vfo_signal_level() -> f32
display_settings() -> DisplaySettings
pub fft_controls_enabled: bool
```

`set_avg_alpha` retains its existing new-frame-weight semantics: 1 is immediate; lower values add averaging. Its documentation now states this correctly. Root was notified to map its smoothing slider accordingly. `waterfall_every_n` counts accepted spectrum frames rather than UI redraws. Source tuning/sample-rate changes discard incomplete IQ windows and pending half-pairs.

`DisplaySettings` snapshots actual FFT/window/cadence, waterfall depth/speed/visibility, SNR settings, palette, grid, averaging/hold/persistence/fill, and FFT/waterfall ranges. Root now calls its `persist_display_settings` helper after spectrum UI rendering to keep its sidebar and persisted config synchronized with changes made through the spectrum's own menus.

Read-only integration review identified stale VFO metadata outside the Radio tab, a squelch badge still using full-band average, Ctrl-scroll bandwidth edits that were overwritten, and unsynchronized spectrum-menu settings. Findings were sent to root; current root edits refresh passband metadata before detection, use the selected-channel detector for the badge, persist Ctrl-scroll bandwidth, and apply the new settings snapshot. This agent did not edit application/config/radio files in this scoped task.

## Verification

**50 spectrum tests passed, 0 failed** through normal Cargo after the final alpha-rendering fix: `cargo test -p ez-gui --no-default-features --lib spectrum::tests -- --nocapture`. Compilation succeeded; 506 unrelated library tests were filtered out. The new headless egui regression inspects the actual emitted WFM RF rectangle and checks that alpha 22 yields a small premultiplied orange contribution (approximately RGB 22/12/4), preventing the former full-bright RGB contribution.

The earlier 49 spectrum tests also passed using `/tmp/ez-sdr-display-harness.rs` and `/tmp/ez-sdr-display-tests` while unrelated audio interfaces were in progress. That harness imports the actual spectrum, theme, fonts, effects, and demod-mode sources and uses Cargo's exact cached dependency artifacts. Its only local test helper reproduces the application's headless egui runner; no spectrum/DSP behavior is stubbed.

The 12 new regressions verify real 65536-sample acquisition and calibrated tone level; arbitrary odd byte boundaries; bounded long-burst work/newest signal; sample-clock cadence and overlapping windows; invalid/no-op configuration; daemon cadence/malformed frames; display-only SNR time constant; waterfall memory/peak preservation; hidden waterfall/no stale UI rows; tuning reset; sideband overlay geometry; and out-of-channel/wrong-sideband squelch rejection.

`rustfmt` passed for the modified source. Inspection confirms the disabled UI scope contains every FFT size/window control and ends before averaging and other display controls. The focused Cargo run does not establish that the entire application suite or native pointer interaction passed; root owns broader verification.

## Limits

No native window server, physical RF/audio measurement, or pixel-matched SDR++ comparison is claimed. The selected-channel detector is a strongest-bin detector, not calibrated integrated channel-power metrology. Waterfall PNGs intentionally use the bounded display resolution; CSV retains every FFT bin. Oversized input calls intentionally drop intermediate display windows to bound GUI-thread FFT work. Native rendering/performance and complete application integration remain root verification tasks.
