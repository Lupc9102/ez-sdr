# UAT receive implementation session

Harness/model: native Codex session subagent `/root/uat_implementation`, inherited GPT-6 route. No Morph or agy processes were used.

Implemented an actual 978 MHz source through FlightAware dump978-fa's direct JSON TCP output. `ez-gui/src/uat_receiver.rs` validates addresses, address qualifiers, positions, altitude, speed, and headings; receives off the UI thread; reconnects; supports cancellation; and bounds line, queue, and tracking memory. Late events from canceled workers cannot overwrite a new connection or feed disconnected UI state.

`ez-gui/src/adsb_panel.rs` now connects those reports to the existing map/table and preserves partial updates. Missing positions do not create markers; stale positions expire independently of other messages. Non-ICAO identities are separate from registration addresses and skip ICAO enrichment. The 978 selector releases the local SDR and exposes connection address, Connect/Disconnect, status, counters, and copyable decoder setup. Returning to1090 restores the local path. The prior mandatory antenna checklist is now optional. The embedded plane SVG remains, with compass rotation corrected and map latitude projection clamped to Web Mercator bounds.

Root coordinated `app.rs` polling, dispatch guards, source status and non-ICAO telemetry handling, plus the module export. No Cargo dependency changes were needed.

Validation:

- `cargo test -p ez-gui --lib --no-default-features uat_`: 12 passed, including real loopback TCP reconnect, second-report delivery, and cancellation.
- `cargo test -p ez-gui --lib --no-default-features adsb_panel::tests`: 21 passed, including existing headless egui map/list rendering and new report merge, address separation, stale-position, source-release, idle-daemon startup, Stop cancellation, tile byte/dimension limits and retry-backoff checks.
- `rustfmt` run on both implementation files.

Setup and verified limits are documented in `UAT_RECEIVER.md`. Physical UAT reception has not been exercised. dump978-fa is an external required decoder; when sharing one dongle, it must be stopped before1090 can reopen that device. The common table's unsigned altitude representation still clamps below-sea-level heights to zero. Native visual QA is owned by the root task.

Root visual-review follow-up: OSM tiles now use an eight-second request timeout, a one-MiB response cap, validated256×256 PNG dimensions and a four-MiB decoder allocation cap. Decoding runs on the tile worker, corrupt cached files are re-fetched, failed tiles back off from5to60seconds, and speculative prefetch waits for visible tiles. Unavailable tiles draw a subtle grid and loading/offline status; dark-theme tiles have a muted tint. Receiver review also fixed1090 entry retaining radio audio, idle daemon not starting, and generic Stop leaving the UAT connection active.
