# Installed SDR++ Radio control audit

## Target
Read-only audit of the installed SDR++ Radio module and resources to establish control labels, mode conditions, defaults/ranges, and any recoverable ImGui layout constants. Primary artifacts: `/usr/lib/sdrpp/plugins/radio.so`, `/usr/lib/libsdrpp_core.so`, `/home/lupc/.config/sdrpp/radio_config.json`, `/usr/share/sdrpp`, and the known cached `/tmp/opencode/sdrpp_noble_amd64.deb`. No production changes, network retries, broad filesystem crawl, or native-window claims.

## Tasklist
- [x] Read parent tracker and installed profile; establish binary identity/resources — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed)
- [x] Extract exact Radio/demod control labels and mode availability from installed symbols/disassembly — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed).
- [x] Recover supported defaults, ranges, enum choices, DSP semantics and layout evidence; mark uncertainty — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed).
- [x] Send actionable integration evidence and finalize this reference report — native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route (exact provider identifier not exposed).

## Tips
- Pickup 2026-09-23: native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider identifier not exposed. Built-in session harness only, no Morph/agy. Parent owns production integration; DSP and squelch owners work separately.
- Read RADIO_CONTROLS_TASK.md. Installed radio.so is unstripped with build ID `5885a1f8316550d5cadec9685ff5210fef0bec4c`; core build ID `42f8b7de641fcd90f03a93fd19947827a884b498`.
- Saved WFM profile is user state, not necessarily factory defaults: bandwidth150000, deemphasis50us, highPass=false, noiseBlankerEnabled=false, noiseBlankerLevel1, snap100000, squelchMode=off, squelchLevel−100, FMIFNREnabled=false, selectedDemodId1.

## Evidence and scope

Installed package cache: `sdrpp 1.2.1-1517`, radio plugin package timestamp 2026-07-05. The plugin is unstripped. Analysis used installed symbols, decoded ELF constants, and control flow, rather than treating string order as screen order. No matching source was recovered from the known package/resources. Disassembly scratch files are `/tmp/sdrpp-radio-disassembly.txt` and `/tmp/sdrpp-radio-annotated.txt`; instruction addresses below refer to this installed build.

## Mode layout and rates

`RadioModule::menuHandler` starts at `0x540a0`. It calls `ImGui::Columns(4,...,false)` at `0x5411e`, emits two buttons before each `NextColumn`, and restores one column. Actual grid:

| Column 1 | Column 2 | Column 3 | Column 4 |
|---|---|---|---|
| NFM | AM | USB | LSB |
| WFM | DSB | CW | RAW |

| Mode | Internal ID | IF rate (Hz) | Default bandwidth (Hz) | Bandwidth bounds (Hz) | Default snap (Hz) |
|---|---:|---:|---:|---:|---:|
| NFM | 0 | 50,000 | 12,500 | 1,000–50,000 | 2,500 |
| WFM | 1 | 250,000 | 150,000 | 50,000–250,000 | 100,000 |
| AM | 2 | 15,000 | 10,000 | 1,000–15,000 | 1,000 |
| DSB | 3 | 24,000 | 4,600 | 1,000–12,000 | 100 |
| USB | 4 | 24,000 | 2,800 | 500–12,000 | 100 |
| CW | 5 | 3,000 | 200 | 50–500 | 10 |
| LSB | 6 | 24,000 | 2,800 | 500–12,000 | 100 |
| RAW | 7 | member-selected rate | same as rate | locked | 2,500 |

Getter evidence: `0x23600..0x23e40`, `0x245d0`, `0x24820..0x24a20`. NFM button label is NFM, but `getName()` returns `FM`, including its configuration namespace. USB VFO reference enum0, centered modes1, LSB2. RAW disables postprocessing and locks bandwidth.

Bandwidth is `InputFloat`, format `%.0f`, step1/fast100. Snap is `InputInt`, step1/fast100, clamped to minimum1 (`0x548a8`); no explicit positive upper clamp observed.

## Control order and visibility

Order recovered by following jumps after Snap, rather than sorting instruction addresses:

1. Mode grid.
2. Bandwidth.
3. Snap Interval.
4. De-emphasis, where supported.
5. Squelch Mode, followed by Squelch Level for Power or CTCSS Tone for CTCSS (Mute).
6. Noise blanker (W.I.P.), with level slider on the same line, disabled while unchecked.
7. IF Noise Reduction; NFM alone has preset combo on the same line, disabled while unchecked.
8. High Pass.
9. Mode-specific controls below.
10. Received Tone for either CTCSS mode.

| Control | NFM | WFM | AM | DSB | USB | CW | LSB | RAW |
|---|---|---|---|---|---|---|---|---|
| De-emphasis | yes | yes | | | | | | |
| Squelch | yes | yes | yes | yes | yes | | yes | |
| Noise blanker | | | | yes | yes | | yes | yes |
| IF Noise Reduction | yes | yes | | | | | | |
| High Pass | yes | yes | yes | yes | yes | | yes | |
| Low Pass | yes | yes | | | | | | |
| AGC Attack / Decay | | | yes | yes | yes | yes | yes | |
| Carrier AGC | | | yes | | | | | |
| Tone Frequency | | | | | | yes | | |
| Stereo / RDS | | yes | | | | | | |

De-emphasis choices in dropdown order: None, 22us, 50us, 75us. Internal values: None3, 22us0, 50us1, 75us2. WFM defaults to50us, NFM None. Taus are actual seconds22e−6/50e−6/75e−6.

Mode-specific order: AM Carrier AGC → AGC Attack → AGC Decay; USB/LSB/DSB Attack → Decay; CW Attack → Decay → Tone Frequency; NFM Low Pass; WFM Low Pass → Stereo → Decode RDS → RDS Incremental Update → Advanced RDS Info plus region combo.

The core `ImGui::LeftLabel` at `0xec0b0` moves the text down by a style-dependent offset, draws unformatted text, calls `SameLine(0,-1)`, then restores prior Y. There is no fixed label width. `FillWidth` at `0xec110` forwards `GetContentRegionAvail().x` to `SetNextItemWidth`. Other Radio rows subtract cursor X from the initial available content width. Exact absolute pixel placement requires a native render; this audit does not claim it.

## AGC and CW

| Mode | Attack default/range | Decay default/range | Other default |
|---|---|---|---|
| AM / USB / LSB / DSB | 50 / 1–200 | 5 / 1–20 | AM Carrier AGC=false |
| CW | 100 / 1–200 | 5 / 1–20 | Tone800Hz |

Defaults come from `RadioModule::instantiateDemod`: AM boolfalse at `0x45c62`, packed50/5 at `0x45c70..82`; SSB/DSB50/5 at `0x4580c..25`; CW100/5 and800 at `0x45de2..df3`. CW Tone Frequency uses step10/fast100, clamped250–1250 (`0x42d66..8c`). AGC slider formatting is `%.3f`; values are divided by IF sample rate for per-sample coefficients. Labeling these values as milliseconds would be unsupported.

Carrier AGC toggles an AM AGC source/mode: checked=true gives internal mode0; unchecked=false gives mode1 (XOR1 in `AM::showMenu`). Both complex and float AGC states reset on change. This is a source-selection switch, not an additive second gain stage. Existing DSP owner has the run-function evidence for the update law.

## Squelch

Exact dropdown options: Off (`off`, enum0), Power (`power`, enum1), CTCSS (Mute) (`ctcss_mute`, enum3), CTCSS (Decode Only) (`ctcss_decode`, enum4). No DCS or noise-squelch option was found. Default Off. Power threshold slider bounds−100..0, default−100, format `%.3fdB`.

CTCSS Tone selector appears only for Mute. Both CTCSS modes show Received Tone, rendered `%.1fHz` or None. Required tone supports Any (enum−2) and this **51-tone** table (`0x5c060`, size0xcc); the installed table includes150.0:

```text
67.0 69.3 71.9 74.4 77.0 79.7 82.5 85.4 88.5 91.5
94.8 97.4 100.0 103.5 107.2 110.9 114.8 118.8 123.0 127.3
131.8 136.5 141.3 146.2 150.0 151.4 156.7 159.8 162.2 165.5
167.9 171.3 173.8 177.3 179.9 183.5 186.2 189.9 192.8 196.6
199.5 203.5 206.5 210.7 218.1 225.7 229.1 233.6 241.8 250.3 254.1
```

## Noise blanker

The installed label literally says `Noise blanker (W.I.P.)`. The range is1..10. Fresh mode selection resets the stored level to0 (`0x4ef2a`), then calls `setNBLevel` (`0x4f24c`) which clamps1..10; effective absent-config default is1. The initial module constructor value10 is superseded, so reporting10 as the user default would be wrong. Enabled defaultsfalse.

`NoiseBlanker::run` at `0x39a30` uses amplitude, not power:

```text
m = a * abs(x) + (1 − a) * m
ratio = abs(x) / m
output = x / ratio if ratio > level, else x
```

Zero-amplitude samples bypass the average update. `a=500/IFsampleRate` is set during mode selection (`0x4f1fe..240`). Threshold is copied directly into DSP (`0x451fa`), with **no dB conversion**. The slider nevertheless formats `%.3fdB`; preserve this discrepancy explicitly if choosing a clearer local label. It limits detected impulses to the updated running mean, rather than replacing them with literal zeros.

## FM IF noise reduction

NFM options: NOAA APT →9 bins, Voice →15 bins, Narrow Band →31 bins. Internal preset3 →32 bins is forced for WFM. `selectDemod` chooses preset3 for every non-NFM mode (`0x4f276..299`), while availability limits actual processing to FM modes. WFM has only the checkbox; preset combo is gated by selectedDemodId==0 (`0x5498b..993`). Enabled defaultsfalse; per-mode configuration keys `FMIFNREnabled` and `fmifnrPreset`.

`FMIF::initBuffers` at `0x304f0` allocates N-bin FFT/IFFT buffers and N−1 initial zero history samples. The exact float window coefficients are:

```text
w[i] = 0.355768 − 0.487396*cos(2πi/(N−1))
       + 0.144232*cos(4πi/(N−1)) − 0.012604*cos(6πi/(N−1))
```

`FMIF::run` at `0x39860` performs, for every input complex sample: window sliding N samples → forward FFT → magnitudes → largest-magnitude bin → copy only that complex bin into otherwise-zero inverse buffer → inverse FFT → emit center sample at floor(N/2) → clear selected inverse-input bin. N−1 samples persist between blocks. **No normalization divides the FFTW inverse result in this code.** A single-bin analytic inverse at the center is mathematically equivalent, provided normalization and window match. This is peak-bin spectral reconstruction; reducing RF lowpass bandwidth alone does not implement this algorithm. Intended reference clocks are50kHz NFM and250kHz WFM.

## FM audio and RDS

Low Pass defaults true in both NFM (`0x45fbe`) and WFM (`0x36249`). WFM constructor also establishes Stereo=false, Decode RDS=false, Advanced RDS Info=false, RDS Incremental Update=true, regionEurope0. Stereo is an explicit toggle and does not need to be enabled to decode RDS.

RDS Incremental Update, Advanced RDS Info and region selector are shown only while Decode RDS is enabled. Region selector shares the Advanced RDS Info line and offers Europe (`eu`,0) and North America (`na`,1). The information table and symbol diagram require both Decode RDS and Advanced RDS Info. Table row order: PI Code, Country Code, Program Coverage, Reference Number, Program Type, Music. Missing values use placeholders such as0x---- and---. The symbol diagram is below the table. Protocol behavior beyond these UI flags has not yet been fully audited.

## Remaining uncertainty

Native visual measurement, RDS protocol details, low/high-pass complete tap designs, and RAW rate-selection origin remain outside the proven findings above. Signal equivalence needs local tests; binary labels alone do not establish matching DSP. No hardware reception or native screenshot was attempted by this audit.

Global style FramePadding/ItemSpacing, header/toolbar heights and absolute pixel coordinates were not recovered. The label width rule above is confirmed; numerical spacing must remain provisional. All confirmed findings were sent promptly to root, demod and squelch owners.

### Additional base-style evidence

The installed `ImGuiStyle::ImGuiStyle()` at `0x126290` initializes FramePadding=(4,3), ItemSpacing=(8,4), ItemInnerSpacing=(4,4), and CellPadding=(4,2). Constants come from `0x22ac90..0x22acb0`, stored at style offsets0x3c/0x4c/0x54/0x5c. `ThemeManager::applyTheme` at `0xeec40` applies dark-theme colors and zero rounding in its inspected entry path. These are confirmed **base style** values; later global/per-window overrides and effective scaling were not established. They do not establish absolute rendered coordinates or toolbar/header heights.
