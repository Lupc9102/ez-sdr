# ADS-B Emulation Session Summary

Implemented a hardware-free RTL-SDR/ADS-B test path.

- Added a deterministic 1090ES DF17 Uc8 IQ waveform generator and live-style source in `ez-daemon/src/hardware/adsb.rs`.
- Added `tcp-iq` ingestion with rtl_tcp `RTL0` greeting and tuning-command compatibility in `ez-daemon/src/hardware/tcp_iq.rs`.
- Added the `adsb-emulator` binary, plus daemon CLI/configuration support for `--source tcp-iq`.
- Corrected ADS-B magnitude normalization in `PacketPipeline` for Uc8-derived Complex32 samples.
- Added demodulator and PacketPipeline tests proving the generated signal is decoded.
- Documented two-terminal usage and RTL-SDR/rtl_tcp assumptions in `README.md`.

Verification:

- `cargo test -p ez-daemon adsb --lib` — passed.
- `cargo test -p ez-daemon rtl_sdr_style_adsb_iq_reaches_packet_pipeline --lib` — passed.
- `cargo check -p ez-daemon --bins` — passed.
- Loopback TCP tests are present but ignored in restricted environments that deny socket binding; run with `--ignored` where permitted.
