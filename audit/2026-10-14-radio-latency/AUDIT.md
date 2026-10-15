# Radio playback latency — 14 October 2026

The reported symptom was immediate UI feedback followed by about three seconds
of old audio when changing frequency or demodulation mode.

## Confirmed cause

The app converted CPAL's selected output format directly into a StreamConfig,
leaving BufferSize::Default. The PulseAudio backend then accepted the server's
default playback target, which can buffer seconds of audio. The application's
own 200 ms backlog limit does not bound audio already queued in that server.

A silence-only test on the user's real PulseAudio/PipeWire output compared the
old default with the new production configuration at 48 kHz, stereo. CPAL's
callback/playback timestamps reported median latency of **2,004,289 us before**
and **36,296 us after**. Maximum reported values were 2,005,078 and 38,487 us.
The [measurement data](playback-latency.json) records both cases. These are
audio-backend timestamp estimates, not microphone measurements or
measurements of the entire RF-tuning-to-speaker path.

## Change

[playback_stream_config](/home/lupc/Documents/ez-sdr/ez-gui/src/audio_output.rs:254)
requests a callback period near 10 ms, rounded to a power of two and clamped to
the output device's advertised frame limits. At 48 kHz this selects 512 frames;
CPAL's PulseAudio backend uses a two-period buffer target. Outputs with unknown
buffer limits retain their backend default. Device selection, requested sample
rate, sample format and channel layout are preserved.

The earlier observation that RF changes can reopen playback is separate from
this measured backlog. This change fixes the multi-second server buffer without
redesigning stream ownership or weakening stale-audio reset behavior.

## Verification

- Fifteen automated audio-output/configuration tests passed.
- Both opt-in real-driver tests passed using silence only: the default/bounded
  latency comparison and mono/stereo stop/restart/receiver-retirement checks.
- No stream errors were reported in the real-driver latency comparison.
- Formatting, whitespace and Clippy checks passed; optimized GUI build passed.
- The code graph was refreshed with AST-only `graphify update .`.

Detailed console evidence is saved beside this report in playback-latency.log,
audio-tests.log, after-audio-driver.log, clippy.log and release-build.log.
The measured latency remains subject to the selected output device and its
driver; Bluetooth and remote outputs may impose additional buffering.
