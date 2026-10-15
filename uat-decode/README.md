# uat-decode

This crate demodulates 978 MHz UAT FSK and corrects downlink/uplink Reed-Solomon
frames in-process. Provide complex `f32` baseband samples at exactly
2,083,333 samples/s centered on 978 MHz. It accepts arbitrary chunk sizes; call
`reset()` after a capture discontinuity, or use `push_at()` to supply absolute
sample positions and receive gap metadata. It does not resample input or tune an
SDR device.

Downlinks carry corrected payload bytes and an optional parsed ADS-B report;
uplinks carry corrected 432-byte payloads. The sample-rate clock and sample
indices describe the caller's stream. RSSI is calculated from the supplied
complex-sample magnitude and is therefore only comparable after consistent
input scaling.

The FEC implementation's licensing and upstream origin are documented in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
