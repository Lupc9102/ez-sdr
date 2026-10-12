# Field validation gates

Automated RF fixtures and silent audio callbacks cannot certify reception through
an antenna or audible playback. The 11 October 2026 audit found no attached RTL-SDR.
The following gates remain unverified until someone records actual observations.
Do not fill them using synthetic-source results.

## Receiver and audible output

Record the application commit/build, OS, receiver model/serial, antenna, driver and
audio device, UTC interval, frequency, sample rate, gain and all DSP settings.

- Confirm device discovery, permissions and reception on the target machine.
- Tune a known local FM station. Listen through the intended output device, change
  gain, volume and squelch, and check stereo/RDS when the station supplies them.
  Retain a short recording and its completeness/sample-loss metadata.
- Check another supported local modulation against a known signal. Confirm tuning,
  bandwidth and squelch changes have the expected audible effect.
- Receive 1090 MHz aircraft through a suitable antenna; record message counts,
  positioned aircraft and the receiver location. If UAT is advertised, separately
  install and demonstrate dump978-fa's JSON feed with its own 978 MHz receiver.
- Repeat stop/start and unplug/reconnect, checking responsive controls and accurate
  device errors. Reconnect using the receiver's serial identity.
- Run for the intended demonstration duration, including concurrent listening and
  recording. Record captured/lost samples, audio underruns, recording completion,
  CPU/memory observations and any USB errors. Preserve failures as well as passes.

Pass only when these observations establish usable reception and audible output
on the actual demonstration equipment. A missing receiver leaves this gate open.

## Original off-air 80k Meteor pass

Obtain a complete, original 80 ksym/s OQPSK interleaved recording with provenance
and permission to use it. Retain its hash, I/Q format/amplitude units, RF clock,
satellite, capture interval and receiver settings. Re-encoding a 72k pass is a
synthetic algorithm check and does not meet this gate.

Decode the same sample interval in EZ-SDR and a pinned SatDump build using matched
profile/settings. Retain both command lines, corrected-frame counts and order,
matched packet bytes and reconstructed channel images. Compare pixels only for
matched accepted frames; state missing frames and decoding failures separately.
Document weak/fading portions and avoid presenting partial-cache coverage as a
controlled sensitivity percentage.

## Distribution and supported scope

Validate an extracted release on a clean supported Ubuntu machine using the
included installation and license notices. Check each binary's help/startup,
native library availability and a recorded-input workflow. Local builds do not
prove portable installers or Windows/macOS compatibility.

The documented product scope is one desktop radio chain, native 1090 MHz ADS-B,
external dump978-fa for UAT and imported Meteor files with bounded channel PNGs.
Daemon controls require loopback or a trusted network; remote untrusted access
requires an authenticated proxy. These are declared boundaries, not field tests.
