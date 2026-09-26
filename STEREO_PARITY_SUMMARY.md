# WFM stereo parity

Agent: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider identifier unavailable. No Morph or agy. Date: 2026-09-23.

## Implemented behavior

`Demodulator::demodulate_stereo(&[u8], DemodMode)` returns stereo frames at the configured audio rate. `last_stereo_locked` reports pilot recovery. The existing mono API remains available unchanged.

Local WFM stereo uses the RF channel filter, FM discrimination, a linear-phase FIR multiplex decimator, a 19 kHz pilot PLL, coherent 38 kHz difference-channel recovery, matching channel filtering, independent 50/75 µs de-emphasis, and persistent fractional output resampling. Weak, absent, or lost pilots produce a smooth mono fallback. Effective IQ clocks below 120 kHz, including clocks reduced by optional RF decimation, use the existing mono path and cannot report stereo lock.

Both channels apply the configured audio LPF and the existing HPF, DC blocker, notch, bass/treble, noise blanker, extra gain, and pitch processing with independent stream state. AGC gain is linked across channels to preserve stereo balance. Stereo restores the legacy mono discriminator's gain after internal pilot normalization, avoiding a loudness jump when toggled. Root owns the local-only toggle, persistence, mono recorder/waveform downmix, and explicit interleaved two-channel audio transport. The daemon protocol remains mono.

## Verification

`cargo test -p ez-gui --no-default-features demod::tests --lib`: **42 passed**, including **nine stereo waveform tests**. `cargo test -p ez-gui --no-default-features sdr_panel::tests --lib`: **31 passed**. Scoped formatting and whitespace checks pass.

The stereo tests synthesize FM IQ, rather than injecting decoded audio, and verify:

- Better than 26 dB channel separation for independent 1 kHz left and 2 kHz right tones, arbitrary pilot phases, pilot frequency offsets of +12/−18 Hz, and RF offsets of ±1.5 kHz.
- Identical mono output without a pilot, rejection of a weak pilot, and return to mono after pilot loss.
- Sample-exact whole-buffer versus fragmented-stream behavior with HPF, notch, bass, and treble enabled.
- Exact output duration and correct channel pitch at 48 kHz and 44.1 kHz from 2.048 MHz input.
- Effective-rate fallback for 48 kHz IQ and excessive RF decimation.
- Functional de-emphasis, audio LPF, gain, and linked AGC controls.
- Mono downmix loudness within 10% across stereo toggles on both test tones.

These tests establish synthetic DSP behavior. Real broadcast reception, physical audio output, and throughput measurements remain root's separate acceptance work. No off-air stereo or RDS capability is claimed by this report.
