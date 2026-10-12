# EZ-SDR final hackathon audit — 11 October 2026

The original audit and grades below describe the audited baseline. Subsequent
implementation repairs and fresh verification are recorded in **Implementation
follow-up** at the end; the original evidence has been preserved.

**Honest overall grade: 6.5/10 against the requested unified replacement. Functionality within the implemented scope: 8/10.**

**Ship verdict: not cleared for an unconditional “fully functional replacement” or public binary distribution.** The three workspaces contain substantial working implementations. A scoped demonstration can be credible, with recorded input and clearly stated limits. Distribution licensing, incomplete advertised CLI feeds, and physical reception validation remain open gates.

This grade measures the current project against dump1090, SDR++ and **SatDump's Meteor workflow**, rather than assigning points for visual resemblance or the number of test functions. It does not require unrelated SatDump satellite protocols. It is a reviewer judgment backed by the evidence below, not a laboratory sensitivity certification. Only dump1090 was run as a direct executable comparison in this audit. SDR++ and SatDump comparisons use their primary documented capabilities plus independent DSP/packet/image checks of EZ-SDR; no side-by-side physical listening or SatDump runtime benchmark was performed.

## What was actually audited

The audit started at commit `09512530dd8857899128b6c4f0d79afd093d2bda`. [Baseline metadata](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/baseline.json) records Rust, Git state and hashes of 175 Rust files. The initial checkout already contained 672 tracked deletions and an untracked AGENTS.md. Those changes were preserved; the audit commit includes only the audit's own changes.

Checks cover workspace builds, strict Clippy and rustdoc, default and headless feature configurations, existing RF/DSP/property regressions, explicitly ignored integration checks, real native window/input behavior, a separately launched daemon, dependency advisories and licenses, independent ADS-B waveform comparison, public RDS multiplex decoding, public 72k Meteor RF, image reconstruction, recording continuity and performance measurements. Generated/native/reference artifacts are linked from the machine-readable manifests in this directory.

Heavy workers share CPUs 0–5 of the machine's 12 logical CPUs and use lowered scheduling priority. This bounds **audit work** to approximately 50% of total CPU capacity. Other desktop applications remain outside that restriction. Build/test workers use four-way concurrency. [CPU policy](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/cpu-policy.json).

Evidence classifications matter:

- **Fresh real-process evidence:** native release GUI, synthetic-source daemon REST/WebSocket/recording/shutdown, actual decoder binaries, real silent audio-driver lifecycle.
- **Independent synthetic RF:** externally specified PPM or separately encoded Meteor signals; these exercise RF ingestion and decoding without inserting decoded objects into the GUI.
- **Public off-air input:** a public Meteor 72k recording and an independently decoded RDS multiplex fixture. Public captures do not demonstrate this machine's USB reception chain.
- **Reused evidence:** earlier measurements are identified explicitly and source-hash checked. The long 80k synthetic-pass proof matches the current LRPT source. Older RDS/audio proofs differ in Cargo metadata after the MQTT dependency fix, so those checks are rerun. [Hash comparison](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/reused-evidence.json).
- **Unavailable evidence:** there is no attached SDR receiver, original off-air 80k Meteor pass or completed live RF/audio field session.

## Architecture and functional scope

The workspace has five crates: `dump1090` handles Mode S/Mode A/C parsing and demodulation; `lrpt-decode` handles Meteor synchronization, coding, packet reconstruction and imagery; `ez-proto` defines bounded stream/control framing; `ez-daemon` exposes headless hardware/replay/synthetic processing and network control; `ez-gui` provides the native desktop, Radio DSP, recording, local ADS-B and background Meteor import.

The native Radio worker processes capture blocks independently of GUI repaint. Capture generations, epochs and sample counters prevent old data from being accepted after retunes/restarts. Recording uses independent stream subscribers and completion metadata. The daemon separates control from high-rate output, validates channel coverage and bounds slow-consumer behavior. The tests include continuity across arbitrary partitions, captured-sample accounting, RF calibration, stream gaps, shutdown, reconnects, stale revisions and rejected controls. These are useful correctness checks, beyond a collection of widget-only tests.

The common desktop interface does not make every capability uniform. There is one active desktop listening DSP chain. VFO B is a saved alternate that can be swapped in. Daemon audio has fewer modes and is mono. UAT requires a separate decoder. Meteor is an imported-file workflow. These limits must remain visible in the presentation.

## Grades and comparison

**ADS-B: 7.5/10.** The implemented 1090 MHz path includes native 2.4 MSPS demodulation, message validation, aircraft tracking and map presentation. The independent decoder probe matches upstream when both runs disable CRC correction. Actual native replay creates a positioned aircraft. Remaining concerns are the standalone CLI's incomplete output wiring, missing field sensitivity/receiver validation and narrower distribution maturity. 978 MHz ingests dump978-fa's JSON TCP output; it is not a native Rust UAT I/Q demodulator. [Upstream dump1090](https://github.com/flightaware/dump1090) provides the benchmark for Mode S reception and established feeder/network workflows.

**Radio: 7/10.** Local AM, NFM, WFM, USB, LSB, DSB, CW and RAW have DSP implementations, with stereo/RDS, calibrated channel measurements, scanning, replay, recording and remote controls. This is a useful radio application. SDR++ supplies multiple simultaneous VFOs, a plugin architecture, broad receiver support and established platform workflows; the current desktop does not match that coverage. The daemon lacks DSB/CW and supplies mono audio. Physical reception, listening quality and long-session USB/audio behavior remain unverified here. [SDR++ primary repository](https://github.com/AlexandreRouma/SDRPlusPlus).

**Meteor: 6.5/10 against SatDump's Meteor workflow.** The offline 72k NRZ-M and interleaved 80k OQPSK paths have meaningful packet/image proof, including a full-length independently generated 80k signal. Imported files decode in the real native window. SatDump's Meteor pipelines also expose live processing and product-generation workflows. EZ-SDR currently offers imported CS8/CF32 and reconstructed channel PNGs, with fewer image/product operations and no certified original off-air 80k result. NOAA APT/HRPT/GOES are outside this comparison's requested scope. [SatDump Meteor pipeline definitions](https://github.com/SatDump/SatDump/blob/master/resources/pipelines/Meteor-M.json), [SatDump project](https://github.com/SatDump/SatDump).

The overall rubric is explicit: implemented functionality **8 × 40%**, replacement coverage **5.5 × 25%**, field confidence **5 × 20%**, shipping readiness **6 × 15%** = **6.475**, rounded to **6.5/10**. A hackathon judge could value the integration idea more highly; that would be a different rubric from functional replacement readiness.

## Open findings, in shipping order

### 1. Distribution licensing is unresolved — P1

`cargo deny check` fails because the enabled `rtlsdr_sys` dependency declares GPL-2.0-or-later, outside the current allowlist. Separately, `dump1090` declares MIT OR Apache-2.0 while its source explicitly identifies translated upstream C implementations, including `demod_2400.c`, `cpr.c` and `mode_ac.c`. No root license files are present.

Evidence: [license scan](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/evidence/cargo-deny.log), [manifest](/home/lupc/Documents/ez-sdr/dump1090/Cargo.toml:9), [translated demodulator](/home/lupc/Documents/ez-sdr/dump1090/src/demod.rs:3), [CPR translation](/home/lupc/Documents/ez-sdr/dump1090/src/cpr.rs:1), [upstream license](https://github.com/flightaware/dump1090/blob/master/COPYING).

Resolve provenance, compatible distribution terms, attribution and notices before publishing an archive or binary. Merely adding GPL to the scanner allowlist would not establish that the application's licensing is correct. This audit preserves the failing check instead of silently accepting a licensing policy on the author's behalf.

### 2. The standalone dump1090 CLI advertises inactive raw/SBS feeds — P1

`--net` starts Beast, raw AVR and SBS listeners, but the main loop only calls `broadcast_beast`. The output library has raw/SBS send methods; they are not wired to the executable's decoded-message path. Three clients were connected before synthetic I/Q was supplied. Beast received **2,190 bytes**, raw and SBS each received **0**.

Evidence: [actual-process comparison](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/adsb-comparison.json), [advertised flags](/home/lupc/Documents/ez-sdr/dump1090/src/main.rs:49), [main-loop output](/home/lupc/Documents/ez-sdr/dump1090/src/main.rs:320), [available send functions](/home/lupc/Documents/ez-sdr/dump1090/src/net_io.rs:173).

This is a concrete functional defect in the CLI contract. It does not invalidate the desktop's native aircraft display. Implement and process-test both feeds, or narrow the command's advertised supported outputs. SBS requires correctly populated BaseStation records, rather than a listener that accepts connections indefinitely.

### 3. Physical receiver and audible-output validation are absent — P1 for a live-RF claim

`rtl_test -t` reports no supported receiver. The native app's local RTL selection displays “device 0 is not connected” and remains responsive. That verifies its unavailable-device path. Silent real-driver tests verify callback lifecycle and queue draining, but do not establish that demodulated audio sounds correct through the intended output.

Evidence: [hardware inventory](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/evidence/rtl-hardware-inventory.txt), [native missing-device screenshot](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native/release-no-hardware-error.png).

Before a live hardware demonstration, validate the actual receiver/antenna and output device on the target machine: radio tuning/gain/squelch and recording; a known FM station with stereo/RDS where available; local ADS-B aircraft positions; repeated stop/start and unplug/reconnect; and an extended run with sample-loss/audio-underrun accounting. Antenna reception and USB permissions cannot be inferred from decoder unit tests.

### 4. Original off-air 80k Meteor performance remains unproved — P1 for 80k field reliability

The independently encoded 80k waveform is valuable evidence of algorithmic and streaming correctness, but it re-encodes packet content from a public 72k recording. Its controlled noise/drift does not establish recovery from an original weak/fading 80k pass. Obtain a complete 80k recording and compare corrected frames, packet bytes and images with SatDump over the same sample interval and settings.

### 5. Replacement coverage is deliberately incomplete — P2

- **Radio:** one active desktop listening chain; narrower hardware/plugin coverage; mono daemon audio; no DSB/CW over the current daemon protocol. VFO B is an alternate frequency, not a second concurrent SDR++ VFO. [VFO help](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2196).
- **Meteor:** imported recordings and channel PNG export; no live capture-to-Meteor pipeline or full SatDump-style product/geometry workflow. Supported image height is bounded at **16,384 lines per channel**. [Limit](/home/lupc/Documents/ez-sdr/lrpt-decode/src/image_builder.rs:14).
- **UAT:** external dump978-fa is a runtime requirement for 978 MHz. The native Rust frontend does not demodulate UAT samples. This must be installed and separately demonstrated if advertised.
- **Browser UI:** the daemon APIs work, but the missing web frontend returns **HTTP 404 at `/`**. Do not present an integrated browser application as part of this build.
- **Network trust:** daemon control is unauthenticated. The browser-origin fix blocks foreign pages; native clients without Origin remain allowed. Use the documented loopback/trusted-network deployment scope. REST has no equivalent Host/Origin guard: the actual isolated daemon accepts a rebinding-shaped frequency POST with a foreign Host and matching Origin (HTTP 204, synthetic tuning changes). This is a server-side transport observation; browser DNS and private-network permission behavior were not tested. The WebSocket repair is not complete HTTP authentication or a general DNS-rebinding defense. [REST implementation](/home/lupc/Documents/ez-sdr/ez-daemon/src/web/mod.rs:66).

### 6. Packaging and minor maintenance gaps remain — P2/P3

The local Linux release build succeeds and all three binaries resolve their dynamic libraries here. They are approximately **20.49 MB GUI**, **5.73 MB daemon** and **1.02 MB dump1090**. The release workflow now uploads all three, but it produces Ubuntu binaries rather than a complete portable installer. It does not bundle the native audio/RTL libraries, source/license notices or validated deployment instructions for Windows/macOS. No real GitHub Actions run or external release was dispatched in this audit.

The headless feature build emits three existing audio-output warnings: unused `AtomicU32`, unused `samples`, and unused `record_failure`. They are lower-priority maintainability issues, not observed runtime failures. The final dependency scan retains an unmaintained bincode warning and yanked `yoke-derive 0.8.3`; these should be tracked. The version handshake is actually enforced in both client and server, and the old “decorative handshake” exception rationale was corrected.

## Confirmed repairs made during this audit

- **Foreign-origin daemon WebSockets:** a foreign page's Origin previously received HTTP 101 and could change synthetic frequency. Control and data-plane upgrades now enforce the browser-origin policy; native clients remain supported. Tests cover foreign rejection, allowed loopback, numeric same-endpoint origins, malformed origins and invalid ports. The actual patched daemon returns **403** and leaves tuning unchanged.
- **Unused vulnerable MQTT TLS dependencies:** the publisher configures plain TCP MQTT only. Disabling rumqttc's unused default TLS feature removes `rustls-webpki 0.102.8` and its four advisories. HTTPS dependencies retain their separate current TLS implementation. Stale ignore entries were removed, leaving only the documented bincode maintenance exception. An unsuppressed scan is also retained, so exclusions cannot hide the result.
- **CI and release setup:** Rust 1.92 is pinned, ALSA/PulseAudio/RTL native build dependencies are installed, Cargo commands use the lockfile, optional receiver features are checked, default/headless tests are included, concurrency is bounded and the daemon binary is uploaded with the GUI and CLI.
- **Strict documentation build:** an intra-doc link pointed to an optional backend that was unavailable in the daemon's default build. It now documents that backend without an invalid link.
- **Ignored fractional-clock FFT test:** the old fixture expected an unaliased frequency from a 12,000-bin tone in an 8,192-point transform. At 2,048,003/7 samples/s that tone lies outside Nyquist and aliases to bin 3,808. The fixture now uses in-range bin 1,200 and is enabled in normal tests. Its low-amplitude, fractional-clock, complete-window and arbitrary-chunk assertions remain.
- **Recording reference and drain budget:** the synchronous WAV reference previously treated post-decimation samples as a new 300 kSPS capture, omitting the causal startup interval retained by the real 2.4 MSPS capture clock. It now uses the same physical capture geometry as the worker. A five-second debug tail deadline was also inadequate under load and now allows 60 seconds. Exact PCM equality, complete sample accounting and zero-loss assertions remain. The focused integration run verifies 52,757 PCM samples from 2,637,824 RF samples byte for byte, including a 300 ms GUI/shared-lock stall. These are test-reference repairs, not a claim that the production demodulator was changed.
- **Squelch integration observation:** an applied revision can initially carry silent acquisition/gap-reset audio. The integration helper now waits for actual nonzero PCM with the requested revision within its existing ten-second deadline when squelch opens. Closed-squelch zero-PCM and stale-revision rejection assertions remain.
- **Performance fixture alignment and labels:** two profiler runs showed a one-block WAV/source mismatch. Draining the tail did not resolve it: the isolated rerun's metadata proves WAV starts at source sample 16,384, while raw/CF32 start at zero and all end at the same sample counter. The fixture now stages radio configuration and the new capture generation before its mock device releases the first block, and drains stopped DSP before finalizing recorders. Exact sample/completeness assertions remain. This stages aligned measurement windows; it does not certify real-device startup timing. The output identifies debug versus release mode. Both failed attempts and recording metadata remain in `before-final-rerun`.
- **Audit runner reliability:** exact test paths were checked against the harness inventory; a command selecting zero passing tests fails verification. Commands, selected environment, timings, return codes and log hashes are recorded.

## Independent comparison evidence

### ADS-B: both actual native decoders recover the same frames

An independent Python generator integrates PPM pulses across 2.4 MSPS sample windows, produces CRC-valid unique aircraft addresses and sweeps five amplitudes, eight fractional sample phases and three Gaussian-noise levels with fixed seed 1011. No Rust demodulator/CRC helper is imported. Upstream dump1090-fa was separately built from commit `0339a57b89cd6e61856cbb13ae342c31ae7be5ac`.

With CRC correction disabled in both executables:

- Noise RMS **0**: both recover **40/40** cases.
- Noise RMS **0.01**: both recover **35/40** cases.
- Noise RMS **0.035**: both recover **20/40** cases.
- Total: both recover the **same 95/120** expected frames; neither produces unexpected frame payloads.

An earlier C run with two-bit correction recovered 98, while the Rust CLI's default disables correction. The comparison was rerun with equivalent settings; the 98-versus-95 result is not evidence of a matched-setting sensitivity deficit. The reference executable returns 1 at finite-file EOF because its reader returns without setting the normal-exit flag; its decoded output and diagnostic are retained rather than disguising that exit as a clean pass.

This is a controlled synthetic agreement test, not a receiver/antenna benchmark or exhaustive Mode S sensitivity measurement. [Generator](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/compare_adsb.py), [results](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/adsb-comparison.json).

A second independent known even/odd CPR waveform was replayed through the **actual release GUI**, creating aircraft **40621D**, a reported 38,000-foot altitude, and a valid map position. The UI receives generated I/Q; aircraft objects are not injected. [Reproduction generator](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/generate_native_adsb.py), [fixture provenance](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/adsb-native-fixture.json), [native screenshot](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native/adsb-replay-aircraft.png).

### Meteor: preserve the off-air/synthetic boundary

The public 72k I/Q cache contains **522,059,776 bytes** from a partially available compressed source. The independent full reference has **7,338 CADUs**. The older full-cache run recovered **6,899 CADUs**, compared **6,055,292 packet bytes**, and produced three 1,568×5,320 channel images with maximum pixel error **1/255** against the oracle for matched frames. The cached RF ends before the complete reference; **6,899/7,338 must not be presented as a controlled sensitivity percentage**. The fresh 20-second interval beginning at 180 seconds recovers **175 corrected frames**, compares **153,846 packet bytes**, and reconstructs three **1,568×136** images with maximum pixel error **1/255**. Accepted frame order matches the independent reference; a twenty-second slice is not claimed to recover the entire 7,338-frame pass. [Fresh packet/image result](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/meteor-public-72k/oracle/verification.json).

The independently generated **80k interleaved full pass** lasts **852.90235 seconds** (14.2 minutes), carries all **7,338 reference CADUs**, and uses a 37° carrier phase, 137 Hz offset, +70 ppm clock drift and 30 dB SNR. Both 997- and 16,384-sample caller partitions recover all expected frames. The independent image parser verifies **6,468,172 packet bytes per partition** and reconstructs the products with a maximum error of **1/255**. Its LRPT source hashes still match today. This result is reused explicitly; the multi-gigabyte waveform was not needlessly rerun under the CPU cap. [Long-pass evidence](/home/lupc/Documents/ez-sdr/audit/2026-10-07/repair-synthetic-80k-long-verification-1011.json).

The image oracle uses an independent packet parser, CDS timestamps, Pillow/libjpeg entropy decoding, and no application image-builder, IDCT or sequence-unwrapping implementation. The decoded reference for image comparison contains the accepted matched CADUs, so image agreement does not prove that every reference frame was detected.

An earlier debug-native GUI run imports the existing independent 72k synthetic `.cf32` fixture at 288,000 samples/s, shows background progress and completes one reconstructed APID-64 channel image. It reports 12 successful RS codewords and 4 failures on that short fixture; the failure count is retained in the evidence. [Actual completed decode](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native/meteor-final.png). This verifies the UI/file/decoder handoff; the public RF and independent oracle supply the stronger image-content checks.

The final **release GUI** then imports the actual public twenty-second signed-CS8 slice at 288,000 samples/s. It shows responsive background progress and finishes all three MSU-MR channel images, reporting **700 RS codewords accepted and 28 failed**. This agrees with 175 corrected four-codeword CADUs in the fresh decoder test. The failure counter remains visible and is not converted into a sensitivity percentage. Both full and compact native windows are captured. [Release Meteor completion](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native/release-public-meteor-final.png), [input provenance](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native-public-meteor-input.json), [native actions and binary hash](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native-meteor-release.json).

### Radio/RDS and output driver

A pinned public 192 kHz mono multiplex WAV is decoded and compared with independent Redsea metadata: PI **0x6201**, PTY **14**, across arbitrary partitions. This checks the implemented RDS decoder on public multiplex content; synthetic RF tests separately cover the RF-to-multiplex chain. The silent CPAL/PulseAudio driver check exercises queue draining, stop, restart and old-receiver retirement without audible playback. Neither establishes subjective audio quality through a physical receiver.

## Native UI and daemon behavior

A private Xvfb server and temporary configuration isolate the audit from the user's real saved settings. Native screenshots come from the actual X11/OpenGL application, with software GL, rather than the headless egui renderer. Mouse/keyboard events exercise the receiver bar, workspace switches, file entry, decode and source errors. 1,400×900 and 1,000×700 windows were inspected. Headless render artifacts separately cover more modes, source states and injected aircraft fixtures; they are identified as render fixtures.

A clean first-run release opens stopped. Demo Radio can be started muted and produces spectrum/waterfall activity. The ADS-B replay proves positioned-aircraft ingestion. Meteor completes an imported decode. Missing RTL hardware is reported. [Native manifests](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native-gui.json), [first run](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native-first-run.json), [ADS-B replay](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/native-adsb-replay.json).

The actual release daemon is launched with a synthetic source and ephemeral loopback ports. It creates a 64 kSPS spectrum channel from a 256 kSPS capture, rejects an out-of-coverage retune, writes aligned nonempty CF32, blocks deletion while recording, finalizes recording and permits deletion afterward. Foreign-origin control returns 403. Missing frontend content returns 404. SIGINT shutdown is clean. [Process result](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/daemon-process.json).

## Interpreting failed first attempts

The initial debug workspace invocation used the harness's default concurrency. It reported 889 passing GUI tests, three failures and seven ignored checks; the earlier decoder/daemon suites passed. The daemon-squelch and saturated-MQTT tests subsequently passed in isolation. The recording test again failed its five-second tail deadline and motivated the explicit deadline repair. Original failure logs remain in the evidence directory.

The first strict rustdoc run found the optional-backend link defect; the repaired check passes. The first new Origin unit test found that the HTTP URI library's `port()` returns None for an out-of-range port, making an invalid explicit port appear absent. The validator now checks for an explicit authority suffix and rejects invalid ports. The real foreign-origin exploit was already rejected; this additional regression closes the malformed-port edge case.

An intermediate four-thread release run also stopped a restarted synthetic-daemon recording after a real sample-counter gap: expected 1,458,176, received 1,490,944. The recorder surfaced the loss rather than certifying a complete file. The focused 35-test UI integration run passes after the WAV/squelch reference repairs, including recording restart. The daemon synthetic source and ingestion loop publish without real-time pacing, so this tests burst capacity rather than precisely 2.0/2.4 million samples per wall-clock second. For example, the final process probe writes 44,630,016 CF32 bytes in a roughly 0.3-second recording window despite a nominal 256 kSPS source. The failure is retained as an overload observation, not a demonstrated live 2.4 MSPS deficit. It does not recur in the final four-thread release workspace run. [Synthetic reader](/home/lupc/Documents/ez-sdr/ez-daemon/src/hardware/synthetic.rs:194).

Final results below are the authoritative post-repair checks. Initial failures are not erased or counted as passes.


## Final checks and evidence limits

- **Default release workspace:** 1,799 passing test executions, zero failures, 14 ignored in the normal invocation. **Headless release workspace:** 1,790 passing executions, zero failures, 13 ignored. The dump1090 unit suite is executed in both library and binary targets; these totals are executions, not a claim of that many unique test cases. [Default results](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/full.json), [headless and integration results](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/runtime.json).
- **Build/static gates:** formatting, workspace/all-target Clippy with warnings denied, rustdoc with warnings denied, all-feature/all-target compilation and the locked hardware-enabled release workspace build pass. All three release binaries resolve their dynamic libraries on this Linux host. This does not prove a clean install on another distribution. [Checks](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/quality.json), [binary hashes and dependencies](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/release-binaries.json).
- **Explicit integrations:** native UI previews, loopback TCP I/Q, rigctl lifecycle/round trip, fractional-clock FFT, and silent real-driver stop/restart pass. Public RDS, public 72k Meteor and the independent image oracle pass. The source-hash-verified long 80k result is reused rather than labeled a fresh run. Three additional ignored LRPT corpus checks are not newly executed here; the public RF and long-pass checks cover their principal transport concerns but are not the same invocations.
- **Dependency security:** the unsuppressed final scan reports zero known vulnerabilities. Maintenance/yanked warnings are retained. **License policy still fails** for the enabled GPL dependency. Neither a zero-CVE result nor green unit tests resolves translated-code licensing or unauthenticated network exposure. [Security results](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/security.json).
- **Scope of the final profiler adjustment:** only the ignored benchmark fixture's readiness/draining setup changes after the clean full-suite runs. It is explicitly executed again and passes, followed by formatting, strict workspace/all-target Clippy and headless all-target compilation. Production code, normal-test bodies and release binaries are unchanged by that last adjustment. [Post-fixture checks](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/post-fixture-quality.json).

## Measured performance

The final isolated release benchmark passes every capture-count and completeness assertion: **25,706,496 native RF samples**, **51,412,992 raw bytes**, **205,651,968 CF32 bytes**, **514,130 WAV samples**, and **zero lost radio samples**. Its mock device releases the first block after setup so all measurement windows align. Startup latency, physical USB/audio behavior and long-session stability are outside this measurement.

Across five cases with 120 measured frames each (ten warmup frames excluded), total CPU time for logic, UI drawing and tessellation has p99 values of **2.390 ms** for wide Radio, **1.417 ms** for compact Radio, **3.267 ms** for 2,000 aircraft, **3.923 ms** while dragging their map, and **1.521 ms** for the satellite map. These fit a 16.667 ms CPU frame budget in this run, but are not measured GPU presentation or actual display FPS. Aircraft are display fixtures; fixture cloning and later software rasterization are excluded from the timed frame section.

Native RF processing per 16,384-sample block has p95 **843 µs**, p99 **928 µs**, and maximum **16,238 µs**, versus a nominal **6,827 µs** capture-block interval at 2.4 MSPS. The isolated scheduling outlier exceeds one block interval; buffering absorbs it without measured loss here. The separate one-second synthetic Radio pipeline cases run at least **4.3× real time**. Neither short result proves a full-pass or overnight hardware run. [Detailed CPU distributions](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/performance/performance.json), [commands/results](/home/lupc/Documents/ez-sdr/audit/2026-10-11-final/performance.json).

Recorded-input demonstration readiness and field readiness are different conclusions. The evidence supports a presentation of **Radio, native 1090 MHz ADS-B and offline Meteor import** with clearly identified inputs. It does not clear a public distribution with unresolved licensing, the advertised inactive feeds, or a claim of complete SDR++/SatDump equivalence. Fix those concrete contracts, then validate the target hardware before a live-reception promise.

## Implementation follow-up

Three Luna subagents implemented the CLI, transport/headless and
licensing/packaging repairs; the coordinating agent reviewed and verified the
combined work. These changes address the software findings without replacing the
original audit's field evidence or grade.

- **Finding 1:** `dump1090` now declares `GPL-2.0-or-later AND BSD-3-Clause`,
  with upstream attribution and full license texts. Combined Linux binaries use
  the compatible GPL-3.0-or-later terms; independently authored components retain
  their original dual-license grants. [License overview](../../../LICENSE.md)
  and [notices](../../../LICENSES/THIRD-PARTY-NOTICES.md) explain the scopes.
  The license-policy check now passes; its allowlist is supported by provenance
  and source-distribution changes.
- **Finding 2:** the executable sends accepted messages to Beast, untimestamped
  raw AVR and populated 22-column BaseStation SBS feeds. The real-process
  regression connects all three clients before independently generated I/Q,
  then checks message payloads, callsign, altitude, paired CPR coordinates,
  ground velocity and timestamps. CPR freshness uses capture time during fast
  replay. Mode A/C raw messages use a valid untimestamped prefix; SBS omits
  namespaced non-ICAO addresses it cannot represent without ambiguity.
- **Finding 3 remains open:** the user reports an RTL-SDR v4, but the latest
  device inventory still finds no supported receiver attached to this machine.
  Antenna reception, audible output, reconnect and extended hardware runs require
  actual equipment observations. [Field checklist](../../../HARDWARE_VALIDATION.md)
  and [fresh inventory](fixes/hardware-inventory.json) retain that gate.
- **Finding 4 remains open:** an original off-air 80k Meteor capture and a matched
  SatDump comparison are still required. The user is locating a recording.
  Existing synthetic 80k and public 72k evidence is not relabeled as that proof.
- **Finding 5:** README now states the single active listening chain, mono and
  mode-limited daemon audio, imported-only Meteor workflow/image-height limit,
  external UAT decoder and absent browser frontend. REST and WebSocket requests
  share a Host/Origin guard. A real synthetic daemon rejects rebinding-shaped
  controls before tuning changes; valid local clients still work. Native DNS
  aliases require a proxy with a localhost/numeric upstream Host. The transports
  remain unauthenticated and retain the documented trusted-network scope.
- **Finding 6:** release packaging supplies all three binaries, installation
  instructions, notices, checksums, builder/native-library metadata and matching
  source with locked Cargo dependencies vendored. It rejects uncommitted release
  inputs and rebuilds the hardware/audio feature configuration before bundling.
  Ubuntu 24.04 is pinned; native runtime libraries remain separately installed.
  No portable installer, Windows/macOS validation or external release is claimed.
  Headless warnings are fixed and strict headless Clippy is enforced in CI.
  The yanked `yoke-derive` 0.8.3 is replaced by 0.8.4; the existing bincode
  maintenance exception and required versioned migration are explicitly tracked.

Fresh [quality checks](fixes/quality.json), [default tests](fixes/full.json),
[headless tests](fixes/headless.json), [dependency/license scans](fixes/security.json)
and [daemon process checks](fixes/daemon-process.json) record commands, results
and available log hashes. Default release tests pass 1,804 executions and headless
release tests pass 1,795, with zero failures; 14 and 13 checks respectively
remain ignored in the normal invocations. These are executions, including
duplicated library/binary tests, rather than unique cases. The first
formatting/headless-helper failures remain
in [first-quality](fixes/first-quality/quality.json); they were corrected before
the final checks. [Source fingerprints](fixes/source.json) identify the checked
code. [Graph update](fixes/graph-update.json) records the AST refresh and
preservation of all 672 pre-existing tracked deletions. The untracked AGENTS.md
also remains outside the implementation commits.

[Package validation](fixes/package.json) verifies the local Linux archive built
from implementation commit `5a97b74`: all checksums match, all 184 checked Rust
source/manifest files match the tested workspace, and vendored dependencies resolve
offline with an empty Cargo cache. Packaged CLI help works, native libraries resolve,
and the packaged GUI creates a native window using isolated temporary settings and
software GL. This is host-level package/startup proof, not a clean-machine install
or physical radio/audio test. Final dependency scans report zero known
vulnerabilities and no yanked dependency warning; bincode's unmaintained warning
remains documented. [Scanner versions](fixes/verification-tools.json) identify the
tools used for the refreshed scan.
