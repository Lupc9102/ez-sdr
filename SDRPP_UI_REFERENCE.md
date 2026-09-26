# SDR++ reference evidence and Radio implementation

## Target
Rebuild EZ-SDR's Radio workspace against SDR++ rather than preserving the previous dashboard. The requested final standard is identical control inventory and spacing; this document distinguishes verified measurements from remaining gaps.

## Tasklist
- [x] Inspect installed SDR++ assets, configuration, and compiled control labels — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider model not exposed
- [x] Implement dedicated Radio toolbar and compact sidebar with working source/DSP controls — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider model not exposed
- [x] Verify frequency entry/digit tuning, real egui pointer/wheel/manual-entry events, idle-state preservation, play/stop/audio lifecycle, keyboard mute synchronization, mode filters, and headless egui rendering with eight focused tests — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider model not exposed
- [ ] Read upstream main-window, source-menu, and radio-module source and compare exact placement/spacing.
- [ ] Compare rendered SDR++ and EZ-SDR at equal viewport/UI scale.
- [ ] Implement the missing reference controls and DSP/backend capabilities listed below.
- [x] Remove superseded SdrPanel/listen_header UI while preserving demodulation/frequency helpers and tests — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider model not exposed
- [x] Run surviving demodulation/frequency helper tests after legacy removal (30 passed) — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider model not exposed

## Tips
- Parity continuation 2026-09-23: built-in inherited Codex session subagents and Codex root added DSB/CW, independent RF channel filters, stereo WFM, real RTL device selection/offset tuning, audio output selection, and bounded 65536-bin FFT/display controls. See RADIO_PARITY_SESSION_SUMMARY.md for current verification and limits. Exact provider model identifiers were not exposed; no Morph or agy was used.
- Legacy removal pickup: native Codex session subagent `/root/radio_rebuild`, inherited GPT-6-family route (provider model not exposed). Own `sdr_panel.rs`, `listen_header.rs`, and scoped `lib.rs` export removal. Root removes dead SdrPanel allocation/event handlers in `app.rs`. Verify all consumers before deletion; no new features.
- Native Codex session subagent `/root/radio_rebuild`, inherited GPT-6-family route. Exact provider model identifier is not exposed to this agent; no Morph/ag y used.
- Root owns app integration/theme/mode bar; this agent owns `ez-gui/src/radio_ui.rs`.
- `RadioUi::toolbar(ui)` and `sidebar(ui)` use nonblocking SharedState locks. Root drains `dsp_changed` and calls `apply_advanced()` after the sidebar lock has been released.
- `radio_ui::tune` is `pub(crate)` so spectrum clicks/keyboard use the same path. Normal tuning moves an independent VFO inside the captured band; Center mode and out-of-span hardware tuning change capture center. Replay preserves its declared capture center and playback position; explicit File center edits relabel file metadata. Stopped state is preserved.
- `RadioUi::start_receiver(&mut SharedState)` lets the spectrum empty-state Start control preserve mute preference and enable audio. Active source mute state follows SharedState so keyboard/web changes appear in the toolbar and sidebar. Host edits commit by Enter or the explicit Reconnect button.
- Headless egui tests verify widget construction and controller behavior, not pixel fidelity, audible output, or a live hardware connection.

## Authoritative installed reference

- Executable: `/usr/bin/sdrpp`, linked to `/usr/lib/libsdrpp_core.so`.
- Core config: `/home/lupc/.config/sdrpp/config.json`.
- Radio config: `/home/lupc/.config/sdrpp/radio_config.json`.
- RTL-SDR config: `/home/lupc/.config/sdrpp/rtl_sdr_config.json`.
- Theme: `/usr/share/sdrpp/themes/dark.json`; font: `/usr/share/sdrpp/fonts/Roboto-Medium.ttf`.
- Installed PNG control assets: menu, play, stop, muted, unmuted, center_tuning, normal_tuning.
- Compiled label inventory read from `/usr/lib/sdrpp/plugins/radio.so`, `rtl_sdr_source.so`, and `libsdrpp_core.so` using `strings`. This proves labels exist but does not prove layout positions or source semantics.

Installed profile values: `uiScale: 1.0`, `menuWidth: 300`, `fftHeight: 300`, stored window `1280 × 662`, `fftRate: 20`, `fftSize: 65536`, FFT display range `-120..0 dB`, waterfall on. Radio WFM profile: `150000 Hz` channel bandwidth, `100000 Hz` snap interval, `50 µs` de-emphasis, squelch off. EZ-SDR sidebar uses the verified **300 px** width. Its 40 px toolbar, 20 px controls, 4 px row gaps, and 4×2 px button padding are provisional compact implementation values until source/render comparison is available.

Installed Dark theme uses WindowBg `#0F0F0FEF`, MenuBarBg `#232323FF`, FrameBg `#33353889`, Button `#70707066`, CheckMark/SliderGrab `#3D84E0FF`, PlotLines `#66E5FFFF`, text white and disabled text `#7F7F7FFF`. Root owns final theme adaptation.

## Control inventory and implementation

| Area | SDR++ evidence | Current EZ-SDR mapping / gap |
| --- | --- | --- |
| Toolbar | Menu, play/stop, per-digit frequency control, mute, tuning-mode icons | Working menu collapse, source play/stop, 12-digit Hz tuning and typed MHz/kHz/Hz/GHz input, mute, volume and functional normal/Center tuning. Center/normal modes now use compact drawn glyphs; exact native icon pixels remain unverified. |
| Source | Source selection; RTL-SDR device/sample-rate/Refresh, Gain, Direct Sampling, PPM Correction, Bias T, Offset Tuning, RTL AGC, Tuner AGC | Working daemon, feature-gated local RTL-SDR, file replay, demo; rate/gain; hardware AGC/PPM/Bias T. Async discovery/refresh, unique-serial selection, true librtlsdr offset tuning and Off/I/Q direct-sampling selection are implemented. Physical USB and shutdown timing remain unverified. |
| Core source controls | IQ Correction, Invert IQ, Offset mode, Decimation | Floating-point source DC correction, IQ inversion, filtered decimation with exact effective rates, and checked additive transverter offset presets/manual entry now feed the actual radio path. FFT precedes the digital VFO mixer. ADS-B bypasses these radio controls and raw recordings retain the source stream. Exact SDR++ algorithm/layout equality remains unproven. |
| Radio modes | NFM, WFM, AM, DSB, USB, CW, LSB, RAW | All eight local modes have controls; DSB product detection and CW narrow RF filter/configurable BFO now have signal-level tests. Daemon protocol lacks DSB/CW/RAW audio, so these local-only mode buttons are disabled there. |
| Radio controls | Bandwidth, Snap Interval, De-emphasis, Squelch Mode, Squelch Level, High Pass, Carrier AGC, AGC Attack/Decay, noise blanker, IF Noise Reduction | Independent RF channel width and audio cutoff, mode-appropriate snap defaults, CW tone, power/CTCSS squelch and tone decode, local audio AGC attack/decay, WFM 50/75 µs de-emphasis, high-pass/cutoff, audio noise blanker/level, AM carrier AGC, and FM IF noise-reduction presets are implemented. Exact SDR++ control order and pixel spacing remain unverified. |
| WFM options | Low Pass, Stereo | Stereo multiplex decoding, pilot PLL/lock indication, mono fallback, independent channel processing and exact output clocks, plus incremental RDS/PI/PS/RT decoding with region selection, are implemented and tested with synthetic FM IQ. Audio lowpass has a separate cutoff, but no bypass control. |
| Audio / Sinks | Stream sink, device, sample rate, mute/volume | Async device discovery, stable explicit device selection, supported sample-rate choice, mono/stereo output conversion, callback error reporting, mute/volume and bounded 200 ms backlog. Multiple streams/sink modules remain absent; native audible output is unverified. |
| Display | Show Waterfall, Full Waterfall Update, FFT Hold, FFT Smoothing, SNR Smoothing, FFT Framerate/Size/Window, Zoom, colormap | Real streaming FFT sizes 256–65536, configurable cadence, waterfall visibility, SNR smoothing, window/averaging, min/max, colormap, grid, pause, zoom/reset, peak hold. Functional Full Waterfall Update switches full/partial bounded texture updates. VFO overlay/detector track a separate tuned frequency. Exact reference layout remains unverified. |
| Other installed sections | Recorder, Frequency Manager, VFO Color, Band Plan, Theme, Module Manager, Rigctl Server | Radio exposes a collapsed **SDR++ Modules** inventory. Sinks and Band Plan have dedicated drawers backed by live state; Recorder, Frequency Manager, VFO Color, Theme and Module Manager shortcut to the closest EZ-SDR tools. Rigctl Server now has a dedicated loopback Hamlib/rigctld-compatible drawer and applies `f/F`, `m/M`, `v/V`, and `q` requests on the shared radio state. The remaining entries are still not literal SDR++ plugin implementations. |

Radio source errors stay visible beneath Source. Daemon-only limitations disable local passband/DSP controls rather than imply unsupported wire commands. File replay picker deliberately lists unsigned byte IQ and CF32 formats; the existing replay worker does not correctly interpret signed CS8, while the dedicated offline Meteor decoder owns signed-CS8 decoding.

## Verification and limitations

Legacy UI removal: deleted the obsolete `SdrPanel` implementation and `listen_header.rs` plus its module export. Preserved shared `DemodMode`, `FreqIdInfo`, `identify_frequency`, `suggest_demod_for_freq`, and their dependent frequency data. Removed 10 tests solely for the deleted UI/formatting helpers; all **30** surviving helper tests pass (`cargo test -p ez-gui --lib sdr_panel::tests --no-default-features`). Source search finds no remaining `SdrPanel` or `listen_header` references.

`cargo test -p ez-gui --lib radio_ui::tests --no-default-features -- --nocapture`: **8 passed**, 0 failed (2026-09-23).

Reviewed the root agent's software-rendered actual egui widgets in `/tmp/ez-sdr-previews/radio-1400x900.png`. The initial 83 px label column wrapped “FFT smoothing”; changed it to 96 px, left aligned and nonwrapping with tooltip. Root is correcting spectrum color blending and re-rendering. This is offscreen rendered application evidence, not a native SDR++/EZ-SDR pixel comparison.

Upstream clone into `/tmp/ez-sdr-sdrpp-reference` failed DNS resolution under the sandbox. The single requested read-only network escalation was rejected before execution because automatic approval review could not access its configured model (`403`, unprefixed `gpt-5.6-luna` vs provider-prefixed allowlist). This was an approval infrastructure failure, not a finding that cloning was unsafe. No retry or bypass was attempted. Exact main-window/source/radio source comparison and pixel matching remain unverified, so **literal SDR++ 1:1 completion is not claimed**.
