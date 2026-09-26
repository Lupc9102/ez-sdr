# ADS-B Input Emulation Task

## Target

Determine how ez-sdr receives RTL-SDR data, identify the decoder boundary used for ADS-B/Mode-S, and implement a repeatable emulator that feeds realistic RTL-SDR-style samples through the same port/input path. Verify that ez-sdr reads and decodes the emulated input correctly without requiring hardware.

## Tasklist

- [x] Map the existing RTL-SDR input and ADS-B/Mode-S processing paths. (Codex GPT-5 / agy analysis harness)
- [x] Define a realistic RTL-SDR-compatible ADS-B test stream and emulator interface. (Codex GPT-5 / collaboration harness)
- [x] Implement the emulator and integration/unit coverage at the appropriate boundary. (Codex GPT-5 / collaboration harness — preamble timing fix and PacketPipeline path verified)
- [x] Run focused tests and document usage, limitations, and RTL-SDR assumptions. (Codex GPT-5 / primary harness)

## Tips

- Primary agent harness/model: Codex GPT-5.
- Review agent `/root/review_tcp_source`: Codex GPT-5 (tcp_iq protocol/CLI review; 2026-09-20).
- Preserve existing user changes; avoid destructive git operations.
- RTL-SDR raw IQ is normally delivered as unsigned interleaved 8-bit I/Q over the device/driver boundary; network transport may instead expose raw IQ or demodulated Mode-S bytes depending on the selected input path.
- Codex GPT-5 / collaboration harness: `Demod2400` scans 19 magnitude samples for the 8 us preamble and requires samples 14..18 quiet. The generated preamble must therefore contain 19 samples (high at indices 1, 3, 9, and 12, with a fifth trailing low at index 18) before the 112 PPM data bits begin at index 19. With only 18 samples, the first data symbol contaminates the quiet-preamble check / phase alignment; adding the final low sample makes `generated_uc8_decodes_as_the_example_aircraft` pass.
- The loopback TCP tests are marked `ignored` in restricted environments where binding sockets is denied; run them with `cargo test -p ez-daemon tcp_iq --lib -- --ignored` where loopback is available.
