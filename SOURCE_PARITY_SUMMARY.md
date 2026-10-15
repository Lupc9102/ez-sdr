# RTL source parity completion

Harness/model: native Codex session subagent `/root/meteor_completion`, inherited GPT-6-family route; exact provider model identifier is not exposed. No Morph or agy.

## Changes

`ez-gui/src/source_manager.rs` now exposes:

- `RtlDeviceInfo { index, name, manufacturer, product, serial }` and `label()`.
- Serde/default/equality-enabled `RtlDeviceSelection { index, serial }`.
- Source fields `rtl_device`, `rtl_devices`, `rtl_device_refresh_error`, and `offset_tuning`.
- `refresh_rtl_devices() -> bool`, `poll_rtl_devices() -> bool`, and `rtl_devices_refreshing() -> bool` for one bounded, nonblocking USB discovery operation at a time.
- `select_rtl_device(index) -> Result<(), String>` and `selected_rtl_device()` for cached-list validation and lookup. Root restarts active hardware after selection/options change.

Selection records a unique nonempty USB serial where available. Refresh and startup resolve that serial against current USB indices; missing or ambiguous saved identities error instead of opening another receiver. Devices lacking a unique serial use an explicit USB index. The hardware worker verifies the opened serial again to catch identity changes during startup.

Offset tuning calls `rtlsdr_set_offset_tuning` on the hardware handle. Unsupported tuners report an error; direct sampling plus offset tuning is rejected before worker creation. Direct sampling is configured before tuning HF frequencies. USB buffers reset before reception, and failed/invalid USB reads terminate with a useful error instead of spinning.

DSB/CW select no daemon audio subscription because the current daemon wire protocol has no such modes. Existing replay cancellation, byte integrity, live overflow handling, and source restart semantics remain intact.

## Validation

- `rustfmt --edition 2021 ez-gui/src/source_manager.rs` completed.
- `cargo test -p ez-gui --offline --no-default-features source_manager::tests --lib`: **28 passed**. Eight new tests cover serial reordering, missing/duplicate identities, index validation, asynchronous single-flight refresh with continued sample delivery, discovery failure/disconnection, nonblocking manager drop during discovery, incompatible options, and persistence through restart/serde.
- `cargo check -p ez-gui --offline --no-default-features --features rtlsdr`: passed against installed librtlsdr 2.0.2. Other agents' temporarily unused spectrum items produced unrelated warnings during concurrent work.

No physical USB receiver was opened or tested. Native USB enumeration/open/read calls have platform-dependent latency; discovery never blocks the UI and only one discovery worker can exist per manager, but the underlying C call cannot be forcibly interrupted. Existing synchronous USB Stop timing remains hardware-dependent. Serialless/duplicate-serial devices cannot be distinguished stably across USB index reordering.

IQ inversion, DC/IQ correction, and decimation were deliberately left for coordinated downstream DSP/sample-rate integration, as communicated to root.

## Demo pacing follow-up

Corrected the simulated source to count two bytes per complex I/Q sample, preserve submillisecond pacing with `Duration::from_secs_f64`, and subtract generation/queue time before its cancellable wait. The old calculation slept for twice the signal duration and added processing time, causing systematic audio underruns. All **29 source tests** pass, including duration-unit regression and existing queue/cancellation tests. Device selection also accepts partially populated saved configuration via serde defaults.
