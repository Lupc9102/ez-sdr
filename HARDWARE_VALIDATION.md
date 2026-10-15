# Field validation gates

Automated RF fixtures and silent audio callbacks cannot certify reception through
an antenna or audible playback. The 11 October 2026 audit found no attached RTL-SDR.
The following gates remain unverified until someone records actual observations.
Do not fill them using synthetic-source results.

## Radio, receiver A/B and audible output

Record the application commit/build, OS, receiver model/serial, antenna, driver and
audio device, UTC interval, frequency, sample rate, gain and all DSP settings.

- Confirm device discovery, permissions and reception on the target machine.
- Enable receiver B, tune both receivers inside the shared capture, and verify
  independent mode, squelch and volume. Swap A/B and confirm the displayed
  frequencies and receiver controls follow the selected receiver.
- Tune a known local FM station. Listen through the intended output device, change
  gain, volume and squelch, and check stereo/RDS when the station supplies them.
  Retain a short recording and its completeness/sample-loss metadata.
- Switch between available sound devices during reception. Confirm a disconnected
  or unavailable device produces a visible error and that selecting an available
  device resumes audible playback without changing RF tuning.
- Check another supported local modulation against a known signal. Confirm tuning,
  bandwidth and squelch changes have the expected audible effect.
- Receive 1090 MHz aircraft through a suitable antenna; record message counts,
  positioned aircraft and the receiver location. For UAT, tune the native IQ path
  to 978 MHz at exactly 2,083,333 complex samples/second and demonstrate decoded
  messages. Also test the optional dump978-fa JSON source with its own 978 MHz
  receiver and record the feed endpoint and decoder version.
- Import a Meteor recording, decode it, and export the reconstructed channels.
  Separately capture a live pass, finalize it, and export the finalized channels.
  Confirm visible errors for invalid input and failed export/finalization.
- Repeat stop/start and unplug/reconnect while reception is active. Check responsive
  controls, a visible device error, and recovery using the receiver's serial identity.
- Run for the intended demonstration duration, including concurrent listening and
  recording. Record captured/lost samples, audio underruns, recording completion,
  CPU/memory observations and any USB errors. Exercise receiver A/B during the soak.
  Preserve failures as well as passes.

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

The documented product scope is two listening receivers sharing one desktop capture,
native 1090 MHz ADS-B, native 978 MHz UAT at 2,083,333 samples/second with optional
external dump978-fa JSON compatibility, and live or imported Meteor with finalized
channel PNG export. Daemon controls require loopback or a trusted network; remote
untrusted access requires an authenticated proxy. These are declared boundaries,
not field tests.

The Intel macOS x86_64 DMG is unsigned and unnotarized. First-launch behavior and
native-library loading still need validation on a clean supported Intel Mac; the
release does not claim notarized distribution.
