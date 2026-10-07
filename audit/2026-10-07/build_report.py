"""Generate the audit inventory and source-linked report from reviewed findings.

Source anchors are resolved against the current checkout; a missing anchor is an
error. Reproductions live in Rust examples and ignored acceptance tests, separately
from this descriptive inventory. This script does not test or fix the application.
"""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent
BASE = "25f98f6264a0fcd85058d0ccff25d269ec542048"
findings = []


def finding(ident, priority, title, evidence, trigger, logic, impact, repair, *anchors):
    sources = []
    for file, anchor in anchors:
        lines = (ROOT / file).read_text().splitlines()
        matches = [i + 1 for i, line in enumerate(lines) if anchor in line]
        if not matches:
            raise ValueError(f"Missing source anchor: {file}: {anchor}")
        sources.append({"file": file, "line": matches[0], "anchor": anchor})
    findings.append(dict(id=ident, priority=priority, title=title, evidence=evidence,
                         trigger=trigger, logic=logic, impact=impact,
                         repair_gate=repair, sources=sources,
                         area="Satellite" if ident.startswith("S") else "Radio"))


finding("S01", "P1", "Advertised interleaved 80 ksym/s Meteor transport is missing",
        "Protocol comparison + complete source trace",
        "Import an actual Meteor interleaved 80 ksym/s OQPSK recording using either Meteor preset, which defaults to 80,000 symbols/s.",
        "Recovered symbol signs go directly to hypothesis lanes and Viterbi. The only deinterleaver reverses the four RS codewords after frame synchronization. That is a different layer. No code removes the 8-bit interleaver markers, tracks their 80-bit spacing, or reverses the 36-branch convolutional interleaver. SatDump's 80k profile explicitly enables that stage before Viterbi; its 72k profile uses a different transport chain. Selecting a symbol rate alone cannot supply the missing transport operation.",
        "The advertised interleaved 80k signal cannot be treated as a supported end-to-end mode. Existing generated fixtures encode a simpler, uninterleaved transport and cannot prove this compatibility. This is a protocol gap, not an assertion that every conceivable 80k synthetic waveform must fail.",
        "Add explicit mission/transport profiles. Decode marker-bearing interleaved symbols with streaming state and validate the resulting CADUs against an independently decoded 80k recording. Also validate the separate 72k NRZ-M profile. Compare packet bytes and images, not just lock flags.",
        ("lrpt-decode/src/lib.rs", "fn push_symbols("),
        ("lrpt-decode/src/reed_solomon.rs", "pub fn deinterleave("),
        ("ez-gui/src/decoding_panel.rs", "pub fn symbol_rate(&self)"))

finding("S02", "P1", "OQPSK carrier/timing recovery depends on absolute RF phase",
        "Reproduced at 640 kS/s and 2.048 MS/s",
        "Feed otherwise identical noiseless OQPSK at initial phases 0°, 15°, 30°, 45°, 60°, 90° and 180°.",
        "Timing uses uncorrected I alone. The demodulator combines I at one instant with Q half a symbol later, then applies carrier correction to that combination. An arbitrary RF rotation already mixes the staggered branches before this deskew. A subsequent complex rotation cannot undo the cross-time mixing; at 90° the branch delay orientation is also reversed. Digital bit sign/swap hypotheses after symbol recovery cannot repair those damaged symbol values.",
        "At both sample rates, three clean CADUs yield 12 valid RS codewords at 0°, 15°, 30° and 180°, but zero at 45°, 60° and 90°. A satellite's unknown carrier phase is ordinary input, not an exceptional format. The 2.048 MS/s probe uses continuous-time symbol indexing at 25.6 samples/symbol; it does not round the input clock as the existing fixture does.",
        "Test a full phase sweep with pulse-shaped independent OQPSK, both stagger orientations, frequency offset, fractional sampling and clock drift. Correct carrier/timing recovery in a way valid for staggered branches. Require payload equality across phases.",
        ("lrpt-decode/src/qpsk.rs", "let timing_error = if self.config.oqpsk"),
        ("lrpt-decode/src/qpsk.rs", "pub fn process("))

finding("S03", "P1", "Viterbi metrics saturate and permanently kill the decoder",
        "Long deterministic noise-to-clean reproduction, with fresh-decoder control",
        "A continuous receiver/file contains a long noise interval before or between useful frames.",
        "Path metrics accumulate Hamming costs indefinitely without subtracting the minimum metric. INF is 1,000,000 and states at or above it are skipped. Once every state reaches INF, no transition can revive a finite state. Output then becomes all zero regardless of later clean symbols. Resetting frame synchronization does not reset these metrics.",
        "After 8,192,000 noisy trellis steps, the clean section has 823 errors in 1,920 aligned comparison bits and zero output ones. A fresh decoder receives the identical clean code and has zero comparison errors and 851 output ones. The noisy interval corresponds to about 102.4 seconds at 80k trellis steps/s; this is a constructed counterexample, not a measured off-air failure time.",
        "Normalize finite metrics periodically and retain valid traceback ordering. Require arbitrarily long noisy/weak intervals to recover a subsequent known clean payload without restarting the app. Compare fresh and aged decoders and test multiple chunk partitions.",
        ("lrpt-decode/src/viterbi.rs", "const INF"),
        ("lrpt-decode/src/viterbi.rs", "pub fn push_bits("))

finding("S04", "P1", "First successful ambiguity lane is selected forever",
        "Continuous six-frame phase-slip reproduction + state trace",
        "After three clean CADUs, apply a 180° phase slip to one continuous six-CADU IQ stream.",
        "Once selected_lane is set, only that lane receives subsequent symbols. Other lanes stop advancing, and no frame-loss or BER condition releases the selection. The selected lane is also used as the public frame-lock condition. FrameSync's own reacquisition cannot change the bit hypothesis that feeds it.",
        "The continuous phase-slip stream produces 12 valid RS codewords; the identical six-frame stream without the slip produces 24. The entire later section is lost. A single seam may legitimately corrupt a frame, but staying on a dead hypothesis prevents sustained recovery after the seam.",
        "Expire lane selection on a defined loss/BER criterion and rebuild or continuously advance alternative hypotheses. Test phase slips, fades, polarity/conjugation changes and valid later frames without an external reset. Publish current frame synchronization, not historical selection.",
        ("lrpt-decode/src/lib.rs", "if let Some(selected) = self.selected_lane"),
        ("lrpt-decode/src/lib.rs", "frame_locked: self.selected_lane.is_some()"))

finding("S05", "P2", "Frame search skips an earlier valid marker in favor of a later cleaner one",
        "Direct frame-synchronizer reproduction",
        "An unlocked input chunk holds two CADUs: the first marker has two allowed bit errors, the second is perfect.",
        "find_sync searches for the globally lowest Hamming distance and stops on a perfect match. FrameSync drains everything before that chosen position. It can discard a complete earlier admissible frame even though no bytes are missing from the input.",
        "The probe extracts one frame instead of two. Loss depends on read/chunk boundaries and marker noise, so a clean one-frame-at-a-time round trip misses it.",
        "Search in chronological order with frame/RS validation, or retain candidates until rejected. Require identical recovered frame sequences across chunk partitions containing imperfect and perfect markers, false payload markers and truncated suffixes.",
        ("lrpt-decode/src/frame_sync.rs", "pub fn find_sync("),
        ("lrpt-decode/src/frame_sync.rs", "pub fn push_bits("))

finding("S06", "P1", "Space-packet headers split across transfer frames are discarded",
        "Direct parser reproduction + CCSDS AOS requirement",
        "A packet begins in the last 1–5 bytes of a VCDU packet zone; its remaining primary header arrives in the next frame.",
        "The parser opens a buffer only after it can parse all six primary-header bytes in the current payload. A shorter trailing header is forgotten. A NO_PACKET_START continuation cannot reconstruct it because no packet buffer was opened. Body fragmentation works, masking this boundary case.",
        "Splitting the same valid packet after three bytes yields zero completed packets; splits after six or nine bytes yield one. Ordinary variable-length packet packing eventually crosses this boundary, losing image segments despite valid RS correction.",
        "Keep partial primary headers per virtual channel until the length field is available, then continue the packet body. Test every split position, multiple continuations, frame gaps, idle-only frames and subsequent new-header pointers. Preserve the CCSDS rule that idle-only frames can occur during a fragmented packet.",
        ("lrpt-decode/src/ccsds.rs", "pub fn push_mpdu_payload("),
        ("lrpt-decode/src/ccsds.rs", "while cursor + SPACE_PACKET_HEADER_LEN"))

finding("S07", "P2", "Same-APID packets on different virtual channels overwrite each other",
        "Direct parser reproduction",
        "Two virtual channels simultaneously hold incomplete packets with APID 64.",
        "Continuation contexts are per VCID, but actual buffers are keyed only by APID. Opening channel B's packet replaces channel A's bytes; channel A's continuation then appends to B's buffer. A frame gap on one virtual channel can likewise clear the other's buffer. The top-level decoder does not constrain processing to one configured VCID.",
        "Fragments of AAAA on VCID 1 and BBBB on VCID 2 produce a completed packet with BBAA. This is deterministic cross-channel data corruption. A capture using one VCID avoids the trigger; generic multiple-VCID support does not.",
        "Own pending packet bytes by VCID, or by a full validated channel identity, and deliver APID only after reassembly. Require independent results and gap handling for interleaved channels sharing APIDs.",
        ("lrpt-decode/src/ccsds.rs", "apid_buffers:"),
        ("lrpt-decode/src/ccsds.rs", "vcid_contexts:"))

finding("S08", "P2", "Image origin is the satellite's sequence zero rather than the received pass",
        "Image-builder reproduction + reference comparison",
        "A recording starts partway through an ongoing packet-sequence cycle.",
        "The per-channel offset retains only phase modulo 43. Raster height is computed from the absolute packet sequence and zero-filled up to that row. There is no received first-row origin. SatDump retains first/last received segment bounds and crops output to those bounds. Counting allocated channel height as decoded lines also counts the black prefix and missing rows.",
        "One valid 8-pixel-high segment at sequence 16,300 allocates a 3,040-pixel-high image. Preview scaling and progress then spend most of their area/count on data never received. Multiple sequence rollovers consume the 16,384-row cap sooner because the missing prefix remains part of the allocation.",
        "Retain absolute sequence/time identities for placement but render relative to the received pass bounds. Track valid segments separately from allocated height. Verify late starts, rollover, missing packets, reordering and cross-channel row alignment with an independently decoded pass.",
        ("lrpt-decode/src/image_builder.rs", "let offset = *channel.offset"),
        ("lrpt-decode/src/image_builder.rs", "pub fn total_line_count("))

finding("S09", "P2", "JPEG dequantization retains an extra half-unit in every table entry",
        "Independent Pillow/libjpeg fixture and exact DC-only counterexample",
        "Decode a valid MSU-MR-style segment with nonzero JPEG coefficients at quality 50.",
        "quantization_table computes scale * Q + 0.5 and clamps it, but never rounds/truncates to the integer quantizer. At quality 50, the DC quantizer should be 16; this code uses 16.5. The current all-gray fixture has zero coefficients, so it cannot expose any dequantization error.",
        "A standard DC coefficient of +16 should give every pixel 160: 128 + (16*16)/8. The app gives 161. On 896 independently decoded non-flat pixels, maximum absolute error is 5 and mean error is 1.956473. Libjpeg IDCT rounding can contribute small differences; the exact DC-only counterexample isolates the table error.",
        "Use the mission's integer quantization convention. Test nonzero DC/AC, quality values on both scale branches, run-length/ZRL/EOB cases and orientation against independent JPEG vectors. Retain a stated pixel-rounding tolerance for differing IDCT implementations.",
        ("lrpt-decode/src/image_builder.rs", "fn quantization_table("),
        ("lrpt-decode/src/image_builder.rs", "fn decode_segment("))

finding("S10", "P2", "Lock and decoded-line diagnostics can report success without current reception",
        "Zero-IQ reproduction + diagnostic source trace",
        "Silence is fed to the carrier loop, or a previously selected frame lane stops receiving useful data.",
        "Near-zero symbols have zero phase error, so averaged error can satisfy the lock threshold with no signal energy. frame_locked means selected_lane exists, not that current frames synchronize. lines_decoded sums allocated raster heights, including gaps and the late-start prefix. These are three distinct quantities presented as current decoder progress.",
        "10,000 zero complex samples set Costas lock true. S04 preserves the selected-lane lock state while later frames fail. S08 makes one segment appear as thousands of lines. These diagnostics can hide the other failures during a pass.",
        "Gate carrier lock on energy/SNR and sustained valid loop behavior; expire frame lock on recent validation; count actual valid scanlines/segments separately from output extent. Test noise, silence, stale lock and sparse images with explicit time/observation bounds.",
        ("lrpt-decode/src/qpsk.rs", "pub fn is_locked("),
        ("lrpt-decode/src/lib.rs", "let progress = DecodeProgress"))

finding("S11", "P2", "Daemon LRPT accepts unusable sampling and cannot select the 72k profile",
        "Daemon configuration source trace",
        "A client subscribes to LRPT telemetry with a narrow channel, or expects a 72 ksym/s signal.",
        "DaemonState fixes LRPT symbol rate at 80,000. Channel validation checks source geometry but has no LRPT minimum sample-rate/profile check. Channel bandwidth controls output sampling, so an otherwise accepted subscription can undersample the symbol stream. Telemetry resets stream DSP at discontinuities but preserves accumulated image state; a retune to a different pass also needs an explicit product boundary.",
        "The API can create a decoder that is incapable of meeting the sampling requirement. Unlike the offline GUI's validate_rates check, this path provides no early configuration error or 72k selector. Normal Meteor UI currently uses offline imports, so this is daemon telemetry scope.",
        "Make transport/symbol rate explicit in telemetry configuration, enforce the demodulator's real sampling and occupied-band requirements, and distinguish capture gaps from new satellite/product sessions. Reject unsupported subscriptions before allocating threads.",
        ("ez-daemon/src/state.rs", "const LRPT_SYMBOL_RATE_HZ"),
        ("ez-daemon/src/state.rs", "fn validate_spec("),
        ("ez-daemon/src/pipelines/telemetry.rs", "fn process_block("))

finding("R01", "P1", "Listen-tab CS8 replay interprets signed bytes as unsigned IQ",
        "Real SourceManager file-replay acceptance probe",
        "Open a .cs8 file through the Listen source picker, which advertises this extension.",
        "Format dispatch recognizes only CF32/fc32 specially; all other files are copied directly into the CU8 capture data plane. RadioIqProcessor then subtracts the unsigned midpoint. CS8 requires a signed interpretation or a midpoint conversion first. This is not merely a harmless constant phase rotation: the mapping jumps at signed zero.",
        "Signed [0,1,-1,64,-64,127,-128,0] reaches the CU8 consumer as [0,1,255,64,192,127,128,0], instead of [128,129,127,192,64,255,0,128]. Waveform amplitudes, spectra and demodulation are corrupted. Meteor's separate CS8 import correctly uses the signed conversion, so one tab working does not validate the other.",
        "Carry an explicit sample format through capture or convert CS8 before CU8 publication. Compare spectra and demodulated payload/audio for equivalent CU8, CS8 and CF32 files at multiple amplitudes and across zero crossings.",
        ("ez-gui/src/source_manager.rs", "let is_cf32 ="),
        ("ez-gui/src/source_manager.rs", "let chunk = if is_cf32"),
        ("ez-gui/src/radio_iq.rs", "pub fn process("))

finding("R02", "P2", "Local float replay discards CF32 dynamic range before radio DSP",
        "Conversion-to-production-IQ probe + file source trace",
        "Replay weak normalized floating-point IQ or floats exceeding unit amplitude.",
        "SourceManager converts every CF32 component to an unsigned byte before publishing CaptureBlock. The later float demodulation API receives already quantized/clipped values. Correct float handling in the daemon and Meteor import does not rescue this local bridge.",
        "0.0001 and 0.0002 both become byte 128 and the same restored component 0.004687488; 1.2 and -1.3 clip to 255/0. Weak recordings lose their signal variation entirely. Existing round trips start with original CU8 data and therefore cannot detect this lost float precision.",
        "Preserve native floating-point IQ through the local data plane, with format/unit metadata, or explicitly limit float replay to an 8-bit compatibility mode. Validate sub-byte amplitudes and headroom against a float reference, not an 8-bit encoder round trip.",
        ("ez-gui/src/source_manager.rs", "lrpt_decode::cf32_le_bytes_to_uc8"),
        ("lrpt-decode/src/iq_source.rs", "pub fn normalized_component_to_uc8("))

finding("R03", "P1", "Daemon narrow-channel decimation aliases strong adjacent signals",
        "Real Channelizer frequency-response reproduction",
        "Create a 12.5 kS/s virtual channel from 2 MS/s wideband IQ, a decimation of 160.",
        "A fixed 63-tap FIR is used for every decimation. The normalized cutoff is also clamped to at least 0.01; at 2 MS/s this floor is above the 6.25 kHz output Nyquist for this channel. Even without that floor, the fixed length cannot provide narrow enough transitions for arbitrary high decimation.",
        "A wanted 1 kHz, amplitude-0.5 carrier gives output RMS 0.499629. A 13.5 kHz interferer folds to the same output frequency with RMS 0.436185: only about 1.18 dB rejection. A 26 kHz carrier still gives 0.296863. This destroys channel isolation and aliases before every downstream demodulator. The GUI's fixed ~200k audio channel uses a different decimation; these exact figures apply to the supported narrow backend channel configuration.",
        "Design filtering from actual passband, transition width and stopband rejection; use staged/polyphase decimation where needed. Sweep frequencies and source rates, and assert the rejection requirement before downsampling. Test adjacent-channel mixtures, not only wanted tones.",
        ("ez-daemon/src/channelizer.rs", "fn make_decimator("),
        ("ez-daemon/src/channelizer.rs", "const FIR_TAPS"))

finding("R04", "P2", "Daemon playback downsamples mono PCM without an anti-alias filter",
        "Resampler frequency probe and real WFM-pipeline-to-playback probe",
        "The daemon sends audio at ~200 kS/s and the sound device runs at 48 kS/s.",
        "AudioPipeline publishes a demodulated sample for each virtual-channel IQ sample. The GUI's AudioResampler uses linear interpolation but never removes content above the destination Nyquist. WFM de-emphasis is not a brick-wall anti-alias filter. AM/NFM also lack a final destination-rate audio low-pass in this route.",
        "A 49 kHz PCM sine at amplitude 0.5 becomes a 1 kHz audible alias at amplitude 0.410990. Through actual daemon WFM demodulation and de-emphasis, 57 kHz modulation with 12 kHz deviation produces a 9 kHz alias at amplitude 0.004411. The latter is a constructed subcarrier-like input, not a claim about every broadcast station's RDS level.",
        "Low-pass to the destination audio passband before decimation, using a streaming rational-rate converter with known rejection. Test real demodulator output, stereo/RDS-range MPX components, source rates, 44.1/48 kHz output and arbitrary frame splits.",
        ("ez-gui/src/audio_resampler.rs", "pub fn process("),
        ("ez-daemon/src/pipelines/audio.rs", "fn process_block("))

finding("R05", "P2", "Local FM loudness changes with RF sample rate and RF decimation",
        "Identical deviation/tone reproduction at two RF rates",
        "Listen to the same NFM signal after changing source sample rate or local RF decimation.",
        "FM output scales raw phase difference by 0.5 (WFM by its own constant) rather than converting phase difference to deviation using the discriminator's actual clock. Phase difference per sample is proportional to 1/Fs. FM mode disables the audio AGC. The optional FM IF stage intentionally compensates back to this source-rate-dependent convention.",
        "A 1 kHz tone with 2.5 kHz deviation gives amplitude 0.040641 at 192 kS/s and 0.003805 at 2.048 MS/s, a factor 10.68 or about 20.57 dB. The station can become very quiet when selecting a normal high RF sampling rate, even though its RF deviation is unchanged. Daemon FM already uses deviation normalization, so routes behave differently.",
        "Define a physical deviation-to-audio scale and normalize with the actual discriminator clock. Require constant gain, pitch and reported deviation across source rates, RF decimation, IF noise reduction and mono/stereo paths.",
        ("ez-gui/src/demod.rs", "fn demod_fm("),
        ("ez-gui/src/demod.rs", "fn demod_wfm("),
        ("ez-daemon/src/pipelines/audio.rs", "let gain = sample_rate_hz as f32"))

finding("R06", "P2", "USB/LSB power squelch measures a symmetric band instead of the selected sideband",
        "Native PowerMeter acceptance probe + mode routing",
        "Enable power squelch or power-dependent recording while listening to USB/LSB.",
        "PowerMeter tests absolute distance from the VFO against bandwidth/2 for every mode. USB selection actually spans [0, bandwidth]; LSB spans [-bandwidth, 0]. SpectrumAnalyzer correctly renders/measures those asymmetric edges, so the displayed channel and audio gate disagree.",
        "With 3 kHz USB bandwidth at 300 kS/s, a wanted +2197 Hz carrier measures -120 dBFS, while an unwanted -1099 Hz carrier measures -6.021 dBFS. The upper half of the wanted voice band can be muted, while the opposite sideband can open the gate or trigger recording.",
        "Share the demodulator's actual passband geometry with power measurement. Test wanted and opposite tones throughout both SSB sidebands, combined signals, VFO offsets and recording/scanning consumers.",
        ("ez-gui/src/demod_native.rs", "fn process(&mut self, samples:"),
        ("ez-gui/src/demod_native.rs", "let signal = if radio.measure_power"),
        ("ez-gui/src/spectrum.rs", "fn vfo_band_edges("))

finding("R07", "P2", "Narrow power measurements can return silence because no FFT bin fits",
        "Native PowerMeter acceptance probe",
        "Use an allowed 50 Hz CW channel with the VFO between bins of the fixed 4096-point power FFT.",
        "Only bins whose center lies inside the channel contribute. At 300 kS/s, bin spacing is 73.2421875 Hz, larger than a 50 Hz band. If no bin center fits, amplitude remains zero, even with a strong carrier precisely at the VFO. CW's own audio squelch is separate; this finding concerns the RF power measurement/monitoring primitive.",
        "An amplitude-0.5 carrier centered at offset 36.621094 Hz measures -120 dBFS. The error is a hard blind spot, not merely a few dB of FFT scalloping loss.",
        "Measure a filtered/time-domain channel or account for partial bins with suitable resolution and leakage control. Validate off-bin carriers at every supported narrow width, and state whether the metric is peak-bin amplitude or integrated RF power.",
        ("ez-gui/src/demod_native.rs", "let size = 4096;"),
        ("ez-gui/src/demod_native.rs", "if (bin * rate / size as f64 - offset)"))

finding("R08", "P2", "Old capture generations are accepted under a new receiver session's controls",
        "Native Processor acceptance probe",
        "Restart/change source with the same center frequency and sample rate while the old session still has queued blocks.",
        "RadioConfig carries generation, but process rejects only disabled state, center mismatch and rate mismatch. A generation change updates snapshot state and resets continuity; it does not compare the block to the requested radio generation. The accepted block is stamped with the new control version and can reach the new sink/recording consumer.",
        "For requested generation 2 and a generation-1 block, the probe emits 327 audio samples and counts zero skipped RF samples. The geometry-only boundary tests do not exercise this same-geometry stale-session case; an existing reset test even feeds a changed generation while keeping the old desired config.",
        "Reject blocks that do not belong to the active requested capture generation before demodulation/publication. Account for the rejected tail separately. Test rapid same-frequency restarts, format/source changes and simultaneous recorder/audio reopening.",
        ("ez-gui/src/demod_native.rs", "if self.snapshot.generation != block.source_generation"),
        ("ez-gui/src/demod_native.rs", "if !radio.enabled"))

finding("R09", "P2", "AM audio aliases when the selected RF band exceeds output audio Nyquist",
        "Local AM demodulator reproduction at a UI-allowed bandwidth",
        "Use an 8 kHz audio device, AM RF bandwidth 15 kHz and 6 kHz envelope modulation.",
        "Complex channel filtering precedes envelope detection, which is good, but its cutoff is RF bandwidth/2. The envelope is then subsampled before an audio anti-alias filter. The later audio LPF cannot remove a component already folded into baseband. Unlike local FM, AM does not run the pre-decimation audio anti-alias stage.",
        "Amplitude-0.1 modulation at 6 kHz becomes a 2 kHz output alias of amplitude 0.071554. An 8 kHz audio clock is accepted by output selection, and 15 kHz AM bandwidth is allowed by the Listen controls. This can make unwanted modulation audible as an unrelated low-frequency tone.",
        "Low-pass the detected envelope before subsampling, with cutoff tied to the destination/effective output rate. Apply the same requirement to product/SSB routes where configured passbands can exceed audio Nyquist. Test rate/bandwidth combinations and nonlinear envelope harmonics.",
        ("ez-gui/src/demod.rs", "fn demod_am("),
        ("ez-gui/src/demod.rs", "let filtered = if matches!"),
        ("ez-gui/src/radio_ui.rs", "DemodMode::Am => (1_000.0, 15_000.0)"))

finding("R10", "P2", "RAW output has the wrong channel count and timebase for playback/WAV",
        "Output-count reproduction + native publication trace",
        "Select RAW in the local Listen tab and play or record its output.",
        "demod_raw pushes I then Q for every IQ sample. demodulate_complex explicitly bypasses resampling in RAW. Native radio publishes that vector as mono at the sound-device audio rate; only WFM stereo is treated as two-channel audio. The callback then bounds excess backlog by dropping frames, concealing the rate mismatch during live playback.",
        "One second of 192 kS/s IQ yields 384,000 mono samples declared as 48 kHz: eight seconds in a WAV timebase. Live playback discards most samples and interleaves I/Q into one audible channel. Higher RF rates increase the mismatch further. The daemon UI disables RAW, so this finding applies to the local exposed mode.",
        "Define RAW semantics explicitly: properly resampled stereo I/Q audio, an RF-rate IQ export, or an analysis-only mode. Carry correct channels/rate through playback and recording. Require duration and sample-count equality for each supported source clock.",
        ("ez-gui/src/demod.rs", "fn demod_raw("),
        ("ez-gui/src/demod.rs", "let audio = if matches!(mode, DemodMode::Raw"),
        ("ez-gui/src/demod_native.rs", "let mono = if desired.config.stereo"))

finding("R11", "P2", "Reset disables the configured audio high-pass until coefficients are recomputed",
        "Public Demodulator reset reproduction + native lifecycle trace",
        "Reset after configuring an audio HPF; native capture discontinuities/resume reset after configure.",
        "reset clears hpf_alpha_computed to zero while leaving audio_hpf_hz configured. post_process interprets zero as disabled. Native configure recomputes coefficients, but process then calls reset on discontinuity or resumed demodulation. The next configure restores coefficients, limiting the native symptom to the reset/resume block; the standalone API remains disabled until a setter/rate recomputation.",
        "With a 1 kHz HPF and 200 Hz AM tone, amplitude changes from 0.019258 to 0.098395 after reset. This is a temporary passband change/click risk in native listening and a persistent reset-contract bug for direct callers. It is not claimed to disable the filter for the whole native session.",
        "Reset state without erasing configured coefficients, or recompute immediately after reset. Require a reset processor to match a freshly configured processor with every advanced filter active, including the first block after a capture gap.",
        ("ez-gui/src/demod.rs", "self.hpf_alpha_computed = 0.0;"),
        ("ez-gui/src/demod_native.rs", "if !continuous || missing > 0"))

finding("R12", "P2", "Daemon SSB rejection degrades sharply at the actual GUI channel rate",
        "Real AudioPipeline 48k/200k comparative reproduction",
        "Use daemon USB/LSB through the GUI, which requests a ~200 kS/s channel.",
        "The sideband selector uses 257 taps, a fixed 1650 Hz shift and 1450 Hz low-pass at the incoming channel rate. Its transition width grows with rate. Existing sideband rejection tests run at 48 kS/s, substantially below the ~200k GUI route.",
        "At +300/-300 Hz, 48k USB produces wanted RMS 0.326282 and opposite 0.000498 (~56.3 dB rejection). At 200k it produces 0.240538 and 0.079689 (~9.6 dB). Strong lower-sideband voice therefore leaks into USB near the voice-band edge. At 1 kHz the rejection is better; the problem is not a total sideband swap.",
        "Bring SSB to a suitable intermediate rate with anti-alias filtering or scale the filter design to a defined transition/rejection. Sweep the entire voice passband at actual production rates for both sidebands and mixed adjacent signals.",
        ("ez-daemon/src/pipelines/audio.rs", "const SSB_FIR_TAPS"),
        ("ez-daemon/src/pipelines/audio.rs", "fn demod_ssb("),
        ("ez-gui/src/source_manager.rs", "bandwidth_hz: 200_000.min"))

finding("R13", "P2", "Automatic recording/monitoring has an undocumented -90 dB cutoff and no daemon power feed",
        "Trigger and application-call source trace",
        "Enable squelch recording/signal monitoring with a power threshold of -95 dBFS, or use a daemon source.",
        "tick_squelch_record requires squelch_db > -90 in addition to signal > threshold, despite a -100..0 power slider. The caller always takes power from the local native DSP snapshot; with a daemon source that processor is disabled, so it supplies no daemon RF power and the fallback is -120. This must be distinguished from intentionally disabling squelch via its explicit mode.",
        "A valid above-threshold weak signal cannot trigger below -90. Daemon listening cannot drive this monitor from its available remote observations through the current caller. The event count also increments after start even if opening a recording failed, making diagnosis harder.",
        "Use explicit enabled/mode state instead of a magic threshold sentinel. Route acknowledged, fresh daemon RF measurements when supported or disable the feature with a clear reason. Increment recording count only after successful start. Test the full threshold range and both sources.",
        ("ez-gui/src/recorder_panel.rs", "let signal_active = signal_db > squelch_db"),
        ("ez-gui/src/app.rs", "// Squelch-triggered recording tick"))

finding("R14", "P1", "Rejected hardware tuning changes capture metadata anyway",
        "Real backend source trace + fault-injected production-ingestion reproduction",
        "An active RTL-SDR, HackRF or Soapy frequency/sample-rate setter returns an error; TCP command writing can fail similarly.",
        "These setters assign the cached requested value before the fallible hardware/API call. On error there is no rollback. Ingest reports the backend's now-mutated getters as status and uses them for subsequent SampleBlock center/rate, but resets epoch only in the success branch. DaemonState also skips channelizer retuning after the rejected setter, splitting its control state from labeled samples.",
        "A fake source modeling the exact cache-before-error pattern, passed through real ingest::spawn, returns a rejected tune while status and the next block both claim 101 MHz; the epoch does not change and discontinuity is false. Physical samples may still be at the old 100 MHz. This can silently mislabel RF, make digital tuning wrong and produce inaccurate recording metadata. No real receiver rejection was injected in this audit.",
        "Commit applied settings only after successful hardware calls; recover/read back actual state after partial failures. Do not publish normal samples with unverified geometry. Test mutation-before-error sources, not just mocks whose rejecting setters preserve state automatically.",
        ("ez-daemon/src/hardware/rtlsdr.rs", "fn set_frequency("),
        ("ez-daemon/src/hardware/hackrf.rs", "fn set_frequency("),
        ("ez-daemon/src/hardware/soapy.rs", "fn set_frequency("),
        ("ez-daemon/src/ingest.rs", "Err(error) =>"))

finding("R15", "P1", "HackRF callback overflow is silently stitched into continuous capture",
        "Hardware callback-to-ingestion source trace",
        "The bounded HackRF callback queue fills because ingestion/processing cannot keep up.",
        "rx_callback ignores the result of try_send. Dropped transfers are neither counted nor represented in IqSource's clock. read_iq returns the next retained bytes, and ingest constructs a contiguous source_sample_index from retained sample counts. The downstream buses can track their own drops, but cannot detect loss that happened before publication.",
        "Radio DSP, LRPT, and recording consumers receive a false continuous clock across an unknown RF gap. They can carry filter/decoder state across missing samples and mark a recording complete despite acquisition loss. This failure path is source-proven; the audit did not overload a physical HackRF.",
        "Maintain acquisition sample indices/drop counters at the callback boundary, preserve complete sample pairs, and mark a new discontinuity/gap after loss. Require downstream recording completeness and DSP resets to agree with injected callback overflow.",
        ("ez-daemon/src/hardware/hackrf.rs", "let _ = ctx.tx.try_send(data)"),
        ("ez-daemon/src/ingest.rs", "let mut clock = if let Some(clock)"))

finding("R16", "P2", "Soapy timeout is treated as a fatal driver failure on its first occurrence",
        "Official API constant + backend-to-ingestion source trace",
        "SoapySDR readStream returns SOAPY_SDR_TIMEOUT (-1) after its 500 ms timeout.",
        "The code's retry branch handles only zero. Every negative return immediately bails, including the documented timeout -1; ingest then marks disconnected and stops the source. The stated three-timeout retry policy therefore does not apply to the real timeout return. Overflow -4 is also terminal rather than a recoverable gap policy.",
        "A transient delayed read terminates daemon listening instead of exercising the intended timeout retry. RF silence is not itself a reason for an SDR to stop producing IQ; the condition concerns read/driver delays. No physical device latency was measured.",
        "Handle documented timeout/overflow/error codes separately with bounded cancellation-aware retries and explicit discontinuity policy. Inject -1, -4 and real errors through the backend seam and verify connection, clocks and shutdown.",
        ("ez-daemon/src/hardware/soapy.rs", "if samples_read < 0"),
        ("ez-daemon/src/ingest.rs", "source read failed"))

finding("R17", "P2", "Daemon RTL gain control reports success without checking the hardware result",
        "Backend gain source trace",
        "RTL-SDR gain mode or tuner gain application fails, including during startup.",
        "apply_gain discards return codes from rtlsdr_set_tuner_gain_mode and rtlsdr_set_tuner_gain. set_gain stores the requested value and unconditionally returns Ok. Gain selection also reports the requested dB value rather than the selected supported step. The local GUI RTL open path checks these calls; daemon RTL does not.",
        "An acknowledged gain change can leave actual gain/mode unchanged, while status and recording sidecars describe the request as applied. The existing ingestion rejection test cannot catch this because the backend never returns the rejected gain result.",
        "Propagate setter errors, expose actual selected gain/mode, and keep requested/applied states separate. Test unsupported steps, zero/auto semantics and failed mode/gain calls during both startup and live changes.",
        ("ez-daemon/src/hardware/rtlsdr.rs", "fn apply_gain("),
        ("ez-daemon/src/hardware/rtlsdr.rs", "fn set_gain("))

finding("R18", "P2", "rtl_tcp gain is a successful no-op",
        "TCP backend command source trace",
        "Request gain on the daemon's rtl_tcp source.",
        "The frequency and rate setters send rtl_tcp commands, but set_gain only changes gain_db and returns Ok. Startup likewise sends cached frequency/rate but no gain/mode command. The remote RTL tuner therefore never receives the requested gain from this implementation.",
        "The app can display an accepted gain that has not reached the receiver. This applies to rtl_tcp mode; manual metadata for a headerless raw IQ stream has different semantics and is not evidence of a tunable receiver.",
        "Implement the supported rtl_tcp gain mode/value commands with clear lack-of-readback semantics, or reject unsupported gain control. Verify exact wire commands and server behavior with an independent rtl_tcp endpoint.",
        ("ez-daemon/src/hardware/tcp_iq.rs", "fn set_gain("),
        ("ez-daemon/src/hardware/tcp_iq.rs", "self.send_command(0x01, self.frequency_hz"))

finding("R19", "P2", "Daemon narrowband listening uses a wide fixed channel without mode-specific RF selection",
        "GUI subscription and daemon demodulator source trace",
        "Use daemon NFM/AM with strong neighboring carriers.",
        "The GUI requests 200,000 Hz (or source rate if smaller) for every supported audio mode. The daemon FM/AM demodulators consume that entire channel and its mean RF power; they add no NFM/AM channel low-pass matched to the desired station width before nonlinear demodulation. SSB has a separate selector. The GUI correctly disables local bandwidth/de-emphasis/stereo controls for daemon mode, so this is an actual backend capability gap, not a falsely claimed working bandwidth knob.",
        "Neighboring carriers tens of kHz away can capture/distort the NFM discriminator or open squelch, and AM can include multiple stations. A ~200k audio transport rate should not be confused with the wanted RF channel width. R03 and R04 address different aliasing stages.",
        "Separate RF passband width, channel sample rate and audio output rate in the protocol. Apply mode-specific complex channel selection before AM/FM detection, and measure squelch on that selected signal. Test desired/adjacent mixtures at realistic amplitude ratios.",
        ("ez-gui/src/source_manager.rs", "bandwidth_hz: 200_000.min"),
        ("ez-daemon/src/pipelines/audio.rs", "fn demod_fm("),
        ("ez-daemon/src/pipelines/audio.rs", "let mean_power:"))

finding("R20", "P2", "Scanning can wait forever on a frequency a fixed replay capture cannot contain",
        "Scanner-to-tuning reconciliation source trace",
        "Start a scan/bookmark scan extending outside a replay file's fixed RF span.",
        "apply_scanner_tune consumes the requested frequency, while radio tuning reconciliation clamps the actual VFO to the available replay band. The scanner retains its original current_freq_hz. tick_spectrum rejects a mismatched VFO and waits without skipping/rejecting the impossible scan channel.",
        "The scan stops making progress with a generic waiting-for-receiver message even though the requested station is physically unavailable in the file. Freshness and tuner-settle guards are useful and should remain; they need a terminal unsupported-range decision.",
        "Validate scan geometry against fixed capture bounds before starting or skip unavailable bookmarks with an explicit result. Test out-of-span, edge-passband and source-transition cases while preserving fresh-measurement requirements.",
        ("ez-gui/src/app.rs", "fn apply_scanner_tune("),
        ("ez-gui/src/scanner.rs", "if !receiver_ready || spectrum.vfo_frequency_hz()"),
        ("ez-gui/src/radio_ui.rs", "pub(crate) fn reconcile_tuning("))

finding("R21", "P2", "IQ-only recording does not finish when the capture session ends",
        "CaptureRecorder EOF/lifecycle source trace",
        "Record IQ without a duration deadline; the replay reaches EOF or the source stops/errors, with no next block.",
        "CaptureRecorder retains a session only for overflow accounting. Its receive-timeout branch exits only after an explicit stop/deadline. It never checks that the owning CaptureSession is no longer live after queued input drains. RecorderPanel observes progress.stopped, which never becomes true by EOF alone. AudioRecorder has different session-end logic; recording audio simultaneously may end the combined session and hide this IQ-only case.",
        "IQ-only REC can remain active with an increasing wall duration and no new bytes; final metadata is delayed until manual stop. Satellite CF32 uses the same generic capture recorder outside the offline Meteor mode. A new session with matching geometry can also require an explicit recorder session boundary.",
        "On an ended owning session, drain the accepted prefix and finalize with a source-end reason without waiting for another sample. Test finite EOF, read failure, manual source stop, same-geometry restart and empty sessions independently of WAV recording.",
        ("ez-gui/src/capture_recording.rs", "let block = input.recv_timeout"),
        ("ez-gui/src/recorder_panel.rs", "if let Some(recorder) = &self.continuous_iq"),
        ("ez-gui/src/satellite/recorder.rs", "pub fn is_running("))

finding("R22", "P2", "Read cancellation and timed-out hardware commands lack a terminal contract",
        "Blocking-read/control/shutdown source trace + librtlsdr implementation",
        "Hardware stalls while marked streaming, or a control acknowledgement times out before the ingestion read returns.",
        "Running/command flags are checked outside blocking read_iq. HackRF loops on 200 ms timeouts indefinitely while hackrf_is_streaming says true, without reading the stop flag. RTL sync reads use librtlsdr's zero bulk timeout in the inspected implementation. Daemon shutdown awaits the ingestion join. IngestHandle's reply timeout drops the waiter but does not cancel the queued command, which may execute after an error was returned.",
        "A stalled device can keep the local source in Stopping and prevent replacement, or keep daemon shutdown/control pending indefinitely. A previously timed-out tune can later change RF state after the user acts on the error. Local UI shutdown avoids a direct UI-thread join, but that does not make the device read cancellable. These are conditional source-proven failure paths, not a measured physical USB hang.",
        "Use a cancellable bounded read contract and correlate command lifecycle through queued/applied/rejected/cancelled states. Ensure shutdown/control latency is bounded even when the driver keeps reporting streaming without data. Test late acknowledgements and avoid releasing hardware ownership before the read actually ends.",
        ("ez-daemon/src/hardware/hackrf.rs", "while self.byte_buf.len() < want_bytes"),
        ("ez-daemon/src/hardware/rtlsdr.rs", "ffi::rtlsdr_read_sync("),
        ("ez-daemon/src/ingest.rs", "fn request("),
        ("ez-daemon/src/app.rs", "ingest_thread.join()"))


def link(file, line=None, label=None):
    target = str(ROOT / file) + (f":{line}" if line else "")
    return f"[{label or file}]({target})"


intro = """# Satellite decoder and radio listening logic audit — 2026-10-07

The satellite decoder is not ready to be described as reliable Meteor reception. The advertised interleaved 80k transport lacks a necessary stage; clean OQPSK reception depends on arbitrary carrier phase; noise can permanently saturate Viterbi; ambiguity selection does not recover after a phase slip; and valid packet boundaries can lose image data. The Listen path has independent format, selectivity, sampling, session and hardware-state defects. The green baseline is evidence that the existing tests pass their chosen cases, not that these signal chains are correct.

This is a behavioral audit of the current checkout. Production behavior has not been fixed. The only modification to a production source file is a test-only include for four deliberately ignored acceptance probes. Those probes assert the desired behavior and currently fail when explicitly run. Generated examples print measurements from the real implementation. All findings below are open.

Audited base commit: `25f98f6264a0fcd85058d0ccff25d269ec542048`. Pre-existing deletions of cached images/reports and the user's untracked AGENTS.md were excluded from this audit's changes. P1 means high-impact functional failure/corruption requiring priority repair; P2 means a substantial conditional, mode-specific, lifecycle or diagnostic failure. No cosmetic/style findings are counted.

## Scope and signal-chain reconstruction

There are three substantially different chains; passing one does not validate the others.

1. **Local Listen:** Demo/RTL/file source → CU8 CaptureBlock and capture session/clock → independent native radio consumer → optional DC/inversion/power-of-two decimation → digital VFO → mode-specific DSP/advanced filters → sound-device-rate mono/stereo output → bounded callback queue. RF previews go separately to the spectrum; IQ recording uses the original capture, WAV uses native demodulated mono.
2. **Daemon Listen:** physical/replay/TCP source → floating-point SampleBus → virtual-channel NCO/FIR decimator → daemon mono AudioPipeline → network AudioFrame with sequence/clock/squelch state → GUI continuity check and playback resampler → sound output. RF width, IF sample rate and final audio rate are not equivalent. GUI local-only controls are disabled for this source.
3. **Meteor import / daemon telemetry:** CS8/CF32 file reader or virtual-channel float IQ → AGC/input decimation/RRC → OQPSK timing/branch deskew/Costas → bit hypotheses → hard Viterbi → optional NRZ-M → CADU sync → randomizer reversal → four conventional-basis RS(255,223) codewords → VCDU/M-PDU extraction → space packet reassembly → MSU-MR Huffman/dequantization/IDCT → six channel rasters/previews/PNG. The RS byte interleave is present; the 80k convolutional symbol interleaver is absent.

The main Meteor tab is offline-only. Satellite tracking/scheduling is an adjacent control path and does not provide RF truth or validate decoded imagery. NOAA APT, HRPT, GOES and UAT decoding are not claimed implemented by this audit.

## Evidence and method

- Traced actual producers/consumers, clock/rate/format transitions, source acknowledgements, mode controls, continuity resets and recorder ownership. Used Graphify for initial navigation and inspected source for conclusions.
- Ran the existing workspace tests with hardware/audio features disabled and checked default-feature targets. Also checked all daemon hardware feature branches at compile time. These are compilation/software checks; mock RTL tests do not prove physical receiver behavior.
- Added clean-waveform phase/rate probes, long noise-to-clean decoding, fragmented/cross-VCID packets, marker ordering and raster-origin probes. Most channel coding in the IQ fixture uses the application's RS encoder, so its passing results are limited controls, not independent protocol certification.
- Used Pillow/libjpeg for a non-flat 14-MCU JPEG segment and decoded golden pixels, plus an exact DC-only vector. Compared Meteor transport and product conventions with independent decoder sources and AOS rules with CCSDS.
- Exercised actual local/daemon demodulators, real Channelizer and real SourceManager replay. The hardware-error integration probe models the real setters' mutation-before-error pattern through production ingestion; physical FFI calls were not fault-injected.
- Counterexamples check physical invariants: carrier phase independence, noise recovery, chronological frame preservation, virtual-channel isolation, correct signed IQ, Nyquist rejection, sample-count/timebase conservation, passband/gate agreement and applied-state truth.

## Findings
"""

coverage = """## Coverage, including conclusions that did not become findings

### Source, clocks and lifecycle

Reviewed local SourceManager open/start/stop/restart, fixed-file VFO constraints, replay pacing/rewind/EOF, CU8/CS8/CF32 dispatch and amplitude sidecars; native CaptureStream/session generations and bounded fan-out; daemon IqSource ingestion, channelizer reconfiguration and rational CaptureClock; RTL, HackRF, Soapy, rtl_tcp and file backends. Local hardware opening checks configuration errors and stable device identity rather than substituting another receiver. Ordinary replay loops create new epochs; daemon replay keeps samples on either side of a rewind in separate blocks. Bus overflow and recording overflow are generally accounted for after publication; R15 identifies acquisition loss before that accounting layer. Requested rate getters are not independent hardware readback. Local replay's millisecond pacing/minimum delay also limits requested speed precision; this is not presented as a measured real-time guarantee.

### RF and audio DSP

Reviewed local double-precision digital tuning, IQ inversion/DC estimation, staged half-band decimation, RF noise blanker/notch routing, RF band geometry, envelope/discriminator/product/SSB detection, CW BFO/volume/gate, audio AGC/carrier AGC, optional FM IF noise reduction, de-emphasis, audio HPF/notch/shelves/blanker/pitch effects and fractional rate conversion. Local source decimation applies anti-alias filtering per half-band stage; no alias failure was inferred merely because it decimates. Local FM's pre-decimation audio anti-alias stage exists, so R09 specifically concerns the envelope/product routes rather than claiming every local mode lacks filtering. CW and DSB have separate supported controls and daemon mode disables them; R10 does not claim daemon RAW is an enabled GUI button.

Reviewed WFM stereo's pilot recovery, MPX-rate processing, L/R matrix/blend, right-channel post-chain, output channel mapping and fallback to mono. Existing synthetic tests exercise pilot phase/frequency, channel ordering and streaming behavior. These support the tested algorithms, but do not establish off-air stereo separation, sensitivity or calibration. Reviewed RDS 57k pre-de-emphasis tap, band-limited baseband conversion, 16 timing phases, differential biphase detection, CRC/offset words including C-prime, station-change reset, PS/RadioText masks, A/B flag and loss-of-lock aging. Existing shaped-signal, clock-drift, phase/noise and chunk tests are meaningful controls. RDS works through the local route; disabled daemon RDS/stereo controls are treated as limitations, not imaginary working features.

Reviewed CTCSS filtering before downsampling, candidate discrimination, confidence/energy criteria, adjacent-tone rejection, confirmation/hang/release and retune/config reset. Existing tests include all listed tones at 44.1/48k, noisy speech, aliases, ambiguity, wrong tones and exact chunk invariance. No independent field sensitivity/false-open measurement is claimed. Reviewed explicit power/off/CTCSS modes and CW's separate audio gate; R06/R07/R13 describe the distinct native RF-power and recorder paths.

### UI controls, spectrum, scanning and remote controls

Traced Listen mode/rate/volume/mute/output-device/source changes, mode profiles, Advanced-panel propagation and config normalization. Advanced widgets forward the complete config at the end of rendering; the smaller mode-profile tuple alone is not proof that other DSP edits are ignored. Output device identity and requested supported rate are checked, callbacks sanitize/clamp samples, keep complete stereo frames, fill underruns with silence and drop old backlog under load. These callback properties do not fix a producer timebase mismatch such as RAW.

Reviewed FFT window coherent-gain normalization, signed-frequency axis/shift, exact effective rate, VFO passband edges, spectrum observation context, freshness/revisions, GUI preview overflow and scanner settling/dwell/hold/exclusion/memory scan. Scanner deliberately requires complete fresh observations after acknowledged tuning. Those guards prevent stale peaks becoming new hits; R20 covers the impossible-capture wait. Peak-bin dBFS is not calibrated input dBm or integrated noise power. Antenna/S-meter wording and relative displays cannot establish absolute RF calibration without receiver/antenna/noise characterization.

Traced keyboard/bookmarks, rigctl and web actions into common tuning/mode controls and receiver acknowledgement paths; read requests and successful acceptance are distinguished from applied hardware state. This audit does not attempt a full web/security/AI audit, external messaging or antenna/TLE accuracy study. R14/R17/R18/R22 affect the receiver-state truth these controls depend on.

### Recording and satellite integration

Reviewed generic capture recording, raw disk writer, native WAV recorder, CF32 satellite recorder, daemon recording start/stop control, overflow/error prefix finalization, sidecars, byte/sample counts and asynchronous finalization. Files use collision-safe creation and failures preserve/report the actual accepted prefix. Capture geometry/epoch checks generally stop mixed recordings; source-generation identity remains distinct from geometry, as R08/R21 show. WAV downmix is explicitly mono and volume/squelch processing affects recorded audio; no stereo-WAV guarantee is assumed. Disk open/setup can still occur synchronously, and physical disk/USB worst-case latency was not benchmarked in this audit.

Reviewed offline decode import validation, complete sample alignment, CF32 finite checks/amplitude metadata, per-block cancellation, bounded preview forwarding, busy-state ownership, export failure cleanup and collision-safe PNG naming. GUI decode uses small read_exact blocks and requires at least two input samples/symbol. Direct library APIs have weaker input/EOF validation than that GUI wrapper; users of those APIs need their own contract. Satellite scheduler tunes through common control acknowledgement, respects recent manual tuning and reports out-of-capture requests; Doppler display is not evidence that a file has been corrected or that a decoder can track it. Offline Meteor is not turned into a live hardware path by that scheduler.

### Satellite algorithms and product assembly

Reviewed AGC, input decimation/RRC, interpolation/history bounds, timing and phase loops, eight bit hypotheses, convolutional generators/inversion, traceback continuity, NRZ-M state, CADU sizing/buffering/candidate selection, PN convention, RS field/root/basis/interleave/error rejection, VCDU counters and M-PDU pointers, APID buffers, JPEG header/Huffman/run-length/DC prediction/zigzag/dequantization/IDCT, sequence rollover, memory cap and previews/export. Randomizer has an independent prefix vector; the conventional RS basis is consistent with the reference Meteor profile, so no incorrect dual-basis criticism is made. Any uncorrectable RS codeword causes the whole interleaved frame to be rejected, preventing every-fourth-byte corruption from reaching products. CRC/RS protection cannot repair S06/S07 parser ownership errors after successful correction. Idle-only AOS frames may occur during packet fragmentation; retaining an open packet across a valid idle-only frame is not itself a bug.

Image tests with constant gray output do not validate quantization, AC orientation, partial captures or channel alignment. S09 supplies a stronger independent control. Cross-channel image co-registration, sequence reordering/duplicate rollover behavior, independent correction-error vectors and real transport acquisition remain necessary release gates; source review alone does not certify them.

## Why the original test suite missed the main failures

The existing suite is large and has useful guards, but the largest satellite tests construct the same coding conventions they then decode. Several waveform helpers round samples/symbol; most carrier tests cover friendly initial phases. Generated 80k symbols lack the mission interleaver. Short Viterbi cases cannot age metrics to INF, and initial acquisition tests do not require automatic phase-slip recovery. Packet tests fragment bodies after complete primary headers and use different APIDs on simultaneous channels. The JPEG control is zero-AC/zero-DC gray. Daemon SSB rejection is tested at 48k rather than the GUI's 200k. Floating replay tests start with CU8-derived floats, while demodulator weak-float tests bypass the file bridge. Hardware rejecting mocks preserve fields before returning errors, unlike the real setters. Reset-equality tests can compare two processors that both lost the same configured HPF.

These are coverage mismatches, not reasons to discard the good tests. Retain existing controls and add the failed physical invariants and independent vectors.

## Repair order and acceptance gates

1. **Restore trustworthy inputs/state:** R01/R02 and R14/R15/R17/R18/R22. Signedness, clock/epoch/loss accounting and applied tuning/gain must be correct before downstream evidence is reliable. Exercise errors, overflow and session transitions as well as successful steady reception.
2. **Make Meteor a real supported transport:** S01/S02/S03/S04. Add mission profiles and independently compare CADUs for both 72k and interleaved 80k under carrier phase, frequency/clock drift, fade/noise and long duration. A lock LED or app-generated gray image is not the gate.
3. **Preserve/verify decoded data:** S05/S06/S07/S08/S09/S10/S11. Require chunk-independent frame/packet bytes, isolated VCIDs, valid sparse pass bounds, independent non-flat pixels and honest progress/lock state.
4. **Repair listening selectivity and clocks:** R03/R04/R05/R06/R07/R09/R10/R11/R12/R19. Specify passband/stopband, gain and sample-count invariants, sweep production rates and RF/audio modes, and include strong adjacent-channel mixtures.
5. **Finish lifecycle/monitor behavior:** R08/R13/R16/R20/R21. Test rapid restarts, unavailable scan ranges, full threshold range, both local/daemon measurement feeds, transient read errors and recorder EOF with no further IQ.

Release evidence should include an independently decoded off-air Meteor capture per supported profile, known packet/image comparisons, a measured radio passband/alias sweep, actual stereo/CTCSS/RDS recordings, representative RTL/HackRF/Soapy error/overload tests and audio-device/disk lifecycle runs. Those field results were not available here and are not silently replaced by the synthetic controls.

## Verification boundaries

This audit establishes the listed source/protocol defects and reproducible software counterexamples. It does not certify hardware sensitivity, RF calibration, USB timing, audio drivers, on-air satellite compatibility, geographic image products or complete GUI performance. No real SDR was opened, no user recording was uploaded and no external messages were sent. A comprehensive logic audit can be completed while field certification remains unproven; fixing the findings and passing those release gates is a separate task.

The source branches for the named components were examined, including the paths that already have meaningful checks. Uncertain behavior is called out as an unverified release gate rather than represented as a passed requirement. The audit result is failure of functional readiness, not a claim that every subsystem is broken.

## Primary references

- [CCSDS AOS 732.0-B-4](https://ccsds.org/Pubs/732x0b4.pdf), sections 4.1.4.2 and 4.3.2: fragmented packets and per-virtual-channel extraction, supporting S06/S07 and the idle-frame qualification.
- [SatDump Meteor profiles](https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/resources/pipelines/Meteor-M.json), [80k deinterleaver parameters](https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/plugins/meteor_support/meteor/deint.h), and [transport decoder](https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/plugins/meteor_support/meteor/module_meteor_lrpt_decoder.cpp): independent transport/profile comparison for S01; conventional RS basis is intentional.
- [meteor_decode](https://github.com/dbdexter-dev/meteor_decode): independently identifies the interleaved 80k mode; corroboration of S01 rather than current satellite scheduling/frequency information.
- [SatDump MSU-MR assembly](https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/plugins/meteor_support/meteor/instruments/msumr/lrpt_msumr_reader.cpp): received segment bounds and alignment, supporting S08's product comparison. The reproduction itself proves the black-prefix allocation.
- [SoapySDR error definitions](https://github.com/pothosware/SoapySDR/blob/master/include/SoapySDR/Errors.h): timeout -1 and overflow -4, supporting R16.
- [librtlsdr synchronous read implementation](https://github.com/osmocom/rtl-sdr/blob/master/src/librtlsdr.c): zero bulk timeout in the inspected upstream implementation, supporting the conditional read-cancellation concern in R22. The audit did not verify every installed driver fork.

## Reproduce and inspect

Run `python3 audit/2026-10-07/reproduce.py` from any directory. It records command exit codes, preserves complete outputs, and checks measured counterexamples instead of equating successful compilation with correct reception. The four explicitly invoked ignored acceptance tests are expected to fail on the audited code. Normal workspace tests should still pass with those tests ignored. To regenerate the independent JPEG fixture, run `python3 audit/2026-10-07/generate_jpeg_fixture.py` with Pillow available; the small generated fixture and golden pixels are included, so reproduction does not need Pillow.

The machine-readable findings inventory contains exact source anchors and the same repair gates as this report. Evidence files preserve the baseline, feature checks, DSP/parser measurements, long Viterbi control and failed acceptance assertions. Source locations are resolved when this report is generated, and findings refer to the audited implementation rather than proposed fixes.
"""

if __name__ == "__main__":
    counts = {p: sum(f["priority"] == p for f in findings) for p in ["P1", "P2"]}
    inventory = {"date": "2026-10-07", "base_commit": BASE,
                 "scope": "Satellite decoder and complete local/daemon radio listening logic",
                 "production_fixes": False, "counts": counts, "findings": findings}
    (OUT / "findings.json").write_text(json.dumps(inventory, indent=2) + "\n")
    text = intro.replace("## Findings", f"## Findings — {len(findings)} open ({counts['P1']} P1, {counts['P2']} P2)")
    satdump = "https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/"
    references = {
        "S01": "[Meteor profiles](" + satdump + "resources/pipelines/Meteor-M.json), [deinterleaver parameters](" + satdump + "plugins/meteor_support/meteor/deint.h), [transport chain](" + satdump + "plugins/meteor_support/meteor/module_meteor_lrpt_decoder.cpp)",
        "S06": "[CCSDS AOS §4.1.4.2](https://ccsds.org/Pubs/732x0b4.pdf)",
        "S07": "[CCSDS AOS §4.3.2](https://ccsds.org/Pubs/732x0b4.pdf)",
        "S08": "[Independent received-segment assembly](" + satdump + "plugins/meteor_support/meteor/instruments/msumr/lrpt_msumr_reader.cpp)",
        "R16": "[Official Soapy error definitions](https://github.com/pothosware/SoapySDR/blob/master/include/SoapySDR/Errors.h)",
        "R22": "[Inspected librtlsdr synchronous read](https://github.com/osmocom/rtl-sdr/blob/master/src/librtlsdr.c)",
    }
    log_groups = {
        "satellite": ["S02", "S04", "S05", "S06", "S07", "S08", "S09", "S10"],
        "viterbi-noise-recovery": ["S03"],
        "radio": ["R02", "R04", "R05", "R09", "R10", "R11", "R12"],
        "channelizer": ["R03"], "ingest-control": ["R14"],
        "native-acceptance-counterexamples": ["R01", "R06", "R07", "R08"],
    }
    logs = {ident: name for name, ids in log_groups.items() for ident in ids}
    for area in ["Satellite", "Radio"]:
        text += f"\n### {area}\n"
        for f in findings:
            if f["area"] != area:
                continue
            text += f"\n#### {f['id']} · {f['priority']} · {f['title']}\n\n"
            text += f"**Evidence:** {f['evidence']}.\n\n"
            text += f"**Trigger:** {f['trigger']}\n\n{f['logic']}\n\n{f['impact']}\n\n"
            text += "**Repair gate:** " + f["repair_gate"] + "\n\n"
            text += "**Source:** " + ", ".join(link(s["file"], s["line"]) for s in f["sources"]) + ".\n"
            if f["id"] in references:
                text += "\n**Independent reference:** " + references[f["id"]] + ".\n"
            if f["id"] in logs:
                name = logs[f["id"]]
                text += "\n**Recorded reproduction:** " + link(f"audit/2026-10-07/evidence/{name}.log", label=name) + ".\n"
    text += "\n" + coverage
    validation_path = OUT / "validation.json"
    if validation_path.exists():
        validation = json.loads(validation_path.read_text())
        text += "\n## Recorded verification results\n\n"
        text += f"Reproduction runner success: **{validation['success']}**. {len(validation['commands'])} commands and {len(validation['counterexample_gates'])} build/counterexample gates were checked. These gates include successful clean controls, documented defects and the intentionally failing acceptance assertions; they do not mean the application was repaired.\n\n"
        for command in validation["commands"]:
            text += f"- {link('audit/2026-10-07/' + command['log'], label=command['name'])}: exit {command['actual_exit']} (expected {command['expected_exit']}).\n"
        text += "\n" + link("audit/2026-10-07/validation.json", label="Machine-readable validation and source SHA-256 provenance") + ".\n"
    (OUT / "REPORT.md").write_text(text)
    print(f"Generated {len(findings)} findings: {counts}")
