# Audio output parity task

## Target

Implement stable CPAL audio-output device and sample-rate selection in `ez-gui/src/audio_output.rs`. Enumerate plain device metadata for a caller-managed background picker; preserve existing `start` as a default wrapper. Explicitly requested unavailable devices/rates must error without rerouting sound. Support F32/I16/U16 streams and expose callback failures through the existing failure API. Root owns UI/config/app integration.

## Tasklist

- [x] Inspect CPAL capabilities and send exact integration API — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Implement device/rate enumeration and validated stream configuration — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Support selected output streams and callback failure reporting — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Add deterministic device/config/conversion tests and check audio/no-audio builds — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; 12 audio tests, 6 no-audio tests, combined audio+RTL cargo check passed
- [x] Write standalone summary with evidence and remaining hardware validation — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; `AUDIO_PARITY_SUMMARY.md`

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family model route, exact provider ID unavailable. Prior source work is complete in `SOURCE_PARITY_SUMMARY.md`. No agy or Morph.
- Root has the API contract: `AudioOutputSelection { device_id, sample_rate }`; `AudioOutputDeviceInfo { id, label, sample_rates, default_sample_rate, is_default }`; `enumerate_output_devices()`; `AudioOutput::start_with_selection(rx, &selection)`. Selection derives serde/default/equality; a rate of 0 uses device default. Root enumerates off-thread and stops/restarts output after selection changes.
- Avoid actual audio-device calls in unit tests; no native or physical-audio success may be claimed from deterministic tests.
- Follow-up integration scope from root: add `start_with_selection_channels(rx, &selection, input_channels)` and `input_channels()` for mono/stereo input; mono output averages L/R, stereo/multichannel output retains L/R then silences extra channels. Callback backlog is bounded to about 200 ms at the actual output rate, discarding oldest whole input frames without swapping split-chunk stereo channels. Root resets/drains transport on channel/device/rate/tuning changes.
