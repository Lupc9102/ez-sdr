# ADS-B coordinates-to-map audit — 14 October 2026

Two reproduced pipeline defects can produce the reported symptom: a valid
callsign stays in the aircraft list while the map reports zero positioned aircraft.
Both are fixed. The screenshot does not identify which defect occurred during
AUA71V's reception; no recording of that flight was available for replay.

## Findings and fixes

### P1: Valid weak position frames were discarded after CRC validation

The desktop semantic merge rejected messages with normalized signal power below
0.001, even after Demod2400 had validated their CRC and address. The daemon did
not impose that additional rejection. A strong identification message followed
by two weaker valid CPR messages produced three accepted demodulator messages
but no desktop position.

`crc_valid_weak_position_frames_survive_the_semantic_merge` reproduced this before
the fix: the accepted count was three, but `position_valid()` was false. The
fixture reduces the position messages' pulse level to CU8 I=132 over an I=127
baseline. Removing the absolute-power veto allows the accepted coordinates to
reach the map. The test also verifies that damaged CRC parity remains rejected.

Implementation: [desktop semantic merge](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:379).

### P1: A forward capture gap erased a fresh fix and cached CPR messages

Both desktop and daemon treated any missing IQ samples as grounds for clearing
all positions and cached even/odd messages. Callsigns and other measurements
survived. The desktop additionally cleared selection, trails and enrichment when
a native snapshot's discontinuity counter changed.

The desktop and daemon `forward_iq_loss` regression tests failed before the fix:
a valid position at approximately 52.257202, 8.667679 became unavailable after
one missing 16,384-sample buffer followed by an identification message. Losing
samples must discard a partial waveform, but it does not invalidate independent
CRC-validated messages with a known, forward-moving acquisition clock.

The fix preserves fresh coordinates and CPR messages across forward loss within
the same known capture epoch and geometry. It does not refresh position age
when only an identity arrives. Waveform overlap is still reset at every gap.
Unknown or regressed clocks, explicit decoder resets, changed formats/clock
geometry, capture rewinds and new sources discard the old reference. Pairing
still enforces the ten-second window; positions still expire after sixty seconds.

Implementation: [desktop capture handling](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:158),
[daemon capture handling](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:201),
[GUI snapshot handling](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:978).

### P2: Upstream callback gaps could have no visible warning

Native-bus loss counters do not count buffers dropped earlier in the USB/callback
queue. The acquisition clock reports that gap, but the GUI previously warned
only when the downstream loss count was positive. A gap within an unchanged
acquisition now also produces an explicit status warning, without inventing a
sample-loss count.

## End-to-end verification

The native integration regression sends encoded CU8 aircraft messages through
the production CaptureStream subscriber, LocalAdsbWorker, snapshot publication,
CentralApp application and actual AdsBPanel rendering. It asserts a visible
aircraft mesh and callsign after callback loss and after weak valid messages.
It also checks selection/trail retention, new-epoch resets and disappearance of
expired coordinates while identity remains available.

Visual evidence uses a deterministic synthetic aircraft and an offline map grid,
not the photographed flight or a live SDR:

- [Fresh marker after callback loss](/home/lupc/Documents/ez-sdr/audit/2026-10-14-adsb-position/position-after-callback-loss.png)
- [Marker decoded from weak position frames](/home/lupc/Documents/ez-sdr/audit/2026-10-14-adsb-position/position-from-weak-frames.png)
- [Expired position correctly hidden](/home/lupc/Documents/ez-sdr/audit/2026-10-14-adsb-position/expired-position-hidden.png)

Completed validation: 81 GUI ADS-B tests, one additional GUI capture-epoch test,
35 daemon packet tests and all 260 dump1090 library tests passed (377 total).
Both GUI and daemon builds passed. Clippy with warnings treated as errors,
format checking and diff whitespace checking passed. The graph was updated
with `graphify update .` using AST extraction only.

A broader GUI library run was terminated with exit 143 before completion; no
full GUI-suite pass is claimed. The focused suites above completed successfully.

No missing-map defect was reproduced in projection, marker creation, clipping
or display filters once fresh valid coordinates reached the panel. This does
not establish that every possible renderer or hardware failure is absent.
