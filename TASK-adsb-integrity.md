# ADS-B Hardware-Free Integrity Task

## Target

Exercise the repository's hardware-free RTL-SDR ADS-B test path, diagnose and
repair any defects exposed by it, and verify the repaired ADS-B feature
integrity without requiring physical SDR hardware.

## Tasklist

- [x] Locate the hardware-free ADS-B test path and establish its expected execution environment. — `/root`, native Codex harness, GPT-5
- [x] Run the ADS-B integrity path and capture all failures. — `/root`, native Codex harness, GPT-5
- [x] Repair defects within the ADS-B feature scope. — `/root`, native Codex harness, GPT-5
- [x] Re-run relevant checks and record verification results. — `/root`, native Codex harness, GPT-5
- [x] Write the session-end report. — `/root`, native Codex harness, GPT-5

## Tips

- 2026-09-20 — `/root`, native Codex harness, GPT-5: initialized task tracking; inspect this document before continuing work.
- 2026-09-20 — `/root`, native Codex harness, GPT-5: the path is `adsb-emulator` → `tcp-iq --rtl-tcp-header` → daemon packet pipeline, with unit coverage in `hardware/adsb.rs`, `hardware/tcp_iq.rs`, and `pipelines/packet.rs`.
- 2026-09-20 — `/root`, native Codex harness, GPT-5: manual startup exposed a documentation-blocking regression: adding the `adsb-emulator` binary made `cargo run -p ez-daemon -- ...` ambiguous. The ADS-B command now explicitly selects `--bin ez-daemon`.
- 2026-09-20 — `/root`, native Codex harness, GPT-5: the wideband channelizer's full-rate FIR attenuated the PPM waveform, yielding zero tracked aircraft. A full-rate pass-through and channelizer regression test now protect the path; loopback WS verification received `KLM 63EW`.
