# Audio output parity completion

Harness/model: native Codex session subagent `/root/meteor_completion`, inherited GPT-6-family route; exact provider model identifier is not exposed. No Morph or agy.

## API and behavior

Implemented in `ez-gui/src/audio_output.rs`:

- `AudioOutputSelection { device_id: Option<String>, sample_rate: u32 }`, with Default, equality, serde, and defaults for partial saved objects. None means system default; 0 Hz means the selected device's default rate.
- `AudioOutputDeviceInfo { id, label, sample_rates, default_sample_rate, is_default }` and `enumerate_output_devices()`. Root runs enumeration on its background worker. Device IDs use CPAL's stable identifier representation; explicit missing/ambiguous devices never fall back to another output.
- `start_with_selection(rx, &selection)` and existing default `start(rx)` wrapper. Unsupported requested rates return errors rather than silently changing rate. F32, I16, and U16 devices are supported.
- `start_with_selection_channels(rx, &selection, input_channels)` and `input_channels()` support mono or interleaved stereo. Mono duplicates to output channels; stereo averages for a mono device, preserves L/R on stereo devices, and silences additional channels on multichannel devices. Root stops/drains/restarts when the input channel count changes.
- Callback errors set `has_failed()`, make `is_running()` false, and expose a drainable `take_error()` message. Stop/restart resets the failure state.

The callback never waits for a shared input mutex. It bounds queue-drain work and retains about 200 ms at the actual output rate. Under overload it discards oldest whole input frames, including across arbitrary split stereo chunks, so retained audio stays fresh without swapping L/R. Samples are clamped, nonfinite values become silence, and underruns are filled with format-correct silence.

## Verification

- `cargo test -p ez-gui --offline --features audio audio_output::tests --lib`: **12 passed**.
- `cargo test -p ez-gui --offline --no-default-features audio_output::tests --lib`: **6 passed**.
- `cargo check -p ez-gui --offline --features 'audio rtlsdr'`: passed.
- Touched audio/source files formatted with rustfmt.

Tests use synthetic device metadata and stream configurations, actual F32/I16/U16 conversion, an asynchronous simulated callback error, locked shared receivers, mono/stereo/multichannel output conversion, split frames, and overload/frame-alignment fixtures. No tests open a physical audio device; prior permissive tests that called native startup without asserting success were replaced by deterministic checks.

Physical sound output, native device discovery, unplug/replug behavior, and end-to-end stereo listening remain hardware validation tasks. Enumeration advertises common rates and supported range endpoints rather than listing every integer in a continuous device rate range; startup still validates an exact requested rate.
