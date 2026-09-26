# Radio controls integration review

## Scope and identity

Native Codex session subagent `/root/integration_review`, inherited GPT-6 route; the exact provider/model route is not exposed. Built-in session agents were used; no Morph or agy execution. This assignment changed only integration tests and task/report documents. Root owns production app/profile/UI fixes.

## CTCSS integration result

All three tests in `ez-gui/tests/radio_controls.rs` pass. Each uses independently synthesized unsigned-byte NFM IQ at 384003 source pairs/second, a carrier 18 kHz above the capture center, DC correction, factor-four FIR decimation, the exact 96000.75 Hz effective clock, VFO mixing, the production demodulator, and the production CTCSS detector. Both 44100 Hz and 48000 Hz audio output are covered.

- A 100 Hz CTCSS tone survives as the detector input while audio HPF, DC blocker, 100 Hz notch, de-emphasis, pitch, zero demodulator gain, and zero playback volume are applied. The entire detector tap and gate vector equal the unaffected receiver's outputs; muted audible samples remain zero.
- Selected 100 Hz squelch rejects the adjacent 103.5 Hz tone while correctly identifying it, and rejects strong voice-band multi-tone audio with no subaudible tone. Both inputs leave the audible gated output closed throughout.
- A 100 Hz tone ending at 1.1 seconds opens and closes the gate causally. Audio, raw taps, gain vectors, and gated outputs are bit-identical with odd byte chunks of 997 and 32767 bytes. Every block asserts equal audio/tap/gate lengths before multiplication. Final frame counts allow only the one retained streaming interpolation interval after conversion to whole audio frames.

Measured transition fixture (2.4 source seconds):

| Audio rate | Output frames | Gate first opens | Last nonzero gain |
| --- | ---: | ---: | ---: |
| 44100 Hz | 105839 | 0.540385 s | 1.510340 s |
| 48000 Hz | 115199 | 0.539979 s | 1.509937 s |

Exact command:

```sh
set -o pipefail
cargo test -p ez-gui --no-default-features --offline --test radio_controls -- --nocapture 2>&1 | tee /tmp/ez-sdr-radio-controls-tests.log
```

Result: **3 passed, 0 failed**, execution time 1.81 seconds. The initial test run found a fixture assertion that compared fractional expected frame counts; it was corrected to whole emitted frames and the retained streaming interval. No production code was changed to obtain this result.

## Enabled-control throughput

`ez-gui/tests/radio_performance.rs` now additionally covers enabled NFM Voice FMIF, WFM mono FMIF, WFM stereo FMIF, WFM mono RDS, and WFM stereo RDS, each at source decimation one and eight. Every case consumes exactly 2400003 source IQ pairs, representing **1.000000 source second**, and asserts approximately 48000 audio frames. Stereo frames each contain two channels.

Fixture generation and receiver construction are outside the timed section. The timed section includes source DC correction/decimation, FFT65536 at 20 Hz, VFO mixing, demodulation, enabled FMIF work, and raw multiplex extraction plus the decoder for RDS cases. RDS fixtures transmit real differential biphase encoded, CRC-protected groups with an arbitrary symbol epoch/carrier phase, 19 kHz pilot, and audio/stereo interferers. RDS success requires at least eight CRC-valid groups, PI `0x54a7`, complete PS `ASTRA FM`, and synchronization. NR and RDS are measured separately because FMIF can attenuate the broadcast multiplex subcarriers.

Exact command:

```sh
set -o pipefail
cargo test -p ez-gui --release --features rtlsdr --offline --test radio_performance radio_enabled_controls_throughput -- --ignored --nocapture 2>&1 | tee /tmp/ez-sdr-radio-enabled-throughput.log
```

Release result: **1 test passed, all 10 benchmark cases passed**, test execution time 2.03 seconds. Each RDS case decoded **10 CRC-valid groups**, PI `0x54a7`, and the complete PS `ASTRA FM` within its one source second.

| Enabled path | Decimation | DSP time | Realtime factor | Audio frames | MPX samples |
| --- | ---: | ---: | ---: | ---: | ---: |
| NFM Voice FMIF | 1 | 0.087 s | 11.5× | 48000 mono | — |
| NFM Voice FMIF | 8 | 0.077 s | 13.1× | 48000 mono | — |
| WFM FMIF | 1 | 0.104 s | 9.6× | 48000 mono | — |
| WFM FMIF | 8 | 0.103 s | 9.7× | 47999 mono | — |
| WFM stereo FMIF | 1 | 0.129 s | 7.8× | 48000 stereo | — |
| WFM stereo FMIF | 8 | 0.114 s | 8.7× | 47999 stereo | — |
| WFM RDS | 1 | 0.168 s | 6.0× | 47999 mono | 2400003 |
| WFM RDS | 8 | 0.089 s | 11.2× | 47999 mono | 300000 |
| WFM stereo RDS | 1 | 0.194 s | 5.2× | 47999 stereo | 2400003 |
| WFM stereo RDS | 8 | 0.116 s | 8.6× | 47999 stereo | 300000 |

These are single-run throughput observations on the current execution host, not latency or cross-machine performance guarantees. The exact effective input rates are 2400003.000 Hz and 300000.375 Hz for decimation one and eight respectively.

The existing AM selection regression and all eight baseline AM/WFM/CW/WFM-stereo throughput cases also pass after extending the shared fixture. Stereo pilot lock remains true in both baseline stereo cases. Baseline throughput spans 6.2×–15.6× realtime in this run, with 47999 audio frames per one source second.

```sh
set -o pipefail
cargo test -p ez-gui --release --features rtlsdr --offline --test radio_performance -- --include-ignored --skip radio_enabled_controls_throughput --test-threads=1 --nocapture 2>&1 | tee /tmp/ez-sdr-radio-existing-throughput.log
```

Result: **2 passed, 0 failed**, test execution time 1.27 seconds.

## App wiring review

No additional concrete production defect was found in this review. Current app wiring consumes the NFM subaudible tap before playback volume and applies the detector's per-sample gain only in CTCSS Mute mode; Decode Only retains audio. Enabled CTCSS and RDS processing continue without playback. RDS consumes the raw WFM multiplex with its reported exact rate. Mode/source/capture changes reset detector state, and inactive detector displays clear. Source stop versus draining the final captured batch is handled by the app's `has_capture` condition.

The tests call public production DSP APIs and model the app's processing order; they do not instantiate or interact with the native GUI. No USB reception, RF hardware, audio device callback, physical speaker, listening latency, native window interaction, or NR-on RDS decode sensitivity is claimed.

## Formatting and patch checks

```sh
rustfmt --edition 2021 --check ez-gui/tests/radio_controls.rs ez-gui/tests/radio_performance.rs
git diff --check -- ez-gui/tests/radio_controls.rs ez-gui/tests/radio_performance.rs RADIO_CONTROLS_INTEGRATION_TASK.md RADIO_CONTROLS_INTEGRATION_REVIEW.md
```

Both checks pass. Logs are retained at the exact `/tmp` paths listed above.
