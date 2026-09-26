# ADS-B Hardware-Free Integrity Session Report

Date: 2026-09-20

## Objective

Validate the hardware-free RTL-SDR-compatible ADS-B path end to end and repair
defects that prevented ADS-B packets reaching the daemon/browser feature.

## Findings and Repairs

1. The documented daemon command became ambiguous after the ADS-B emulator binary
   was added. `README.md` now explicitly selects `--bin ez-daemon`.
2. A 2.4 MSPS ADS-B virtual channel was still routed through the channelizer's
   guarded anti-alias FIR despite requiring no decimation. That filter attenuated
   Mode S PPM detail and caused the packet pipeline to track zero aircraft.
   Full-rate channels now use a one-tap pass-through decimator.
3. Added a regression test that generates RTL-SDR-style Uc8 ADS-B IQ, sends it
   through the wideband channelizer, and asserts the packet pipeline tracks the
   known emulator aircraft.

## Verification

- `cargo test -p ez-daemon --lib`: 163 passed, 2 loopback-only tests ignored.
- `cargo test -p ez-daemon --lib hardware::tcp_iq::tests:: -- --ignored --test-threads=1`:
  2 passed.
- `cargo fmt --check`: passed.
- `npm test` and `npm run build` in `ez-web`: passed.
- Manual loopback path passed:
  `adsb-emulator` → `ez-daemon --source tcp-iq` → ADS-B REST channel →
  `ws/stream/adsb-packets/1`. The received payload included ICAO `4735190`,
  callsign `KLM 63EW`, and a nonzero message count.

## Agent Record

- `/root` — native Codex harness, GPT-5.
