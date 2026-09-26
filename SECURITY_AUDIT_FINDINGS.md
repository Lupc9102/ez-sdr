# Focused security/web audit (2026-09-13)

Confirmed findings against the current tree. No source files were modified.

## SEC-1 (High): unauthenticated remote control and cross-origin WebSocket hijacking

- `ez-daemon/src/main.rs:43-53` lets the daemon bind both TCP and web listeners to any supplied address; only the CLI default is loopback.
- `ez-daemon/src/web/api.rs:29-66` mounts mutating REST routes (frequency, sample rate, gain, channel creation/deletion, retune, recording) without an authentication or authorization layer.
- `ez-daemon/src/web/ws.rs:130-145,177-205` upgrades `/ws/control`, emits `Welcome`, and immediately accepts `ClientCommand`s without credentials or a version/Hello check. `apply_command` at `ez-daemon/src/server.rs:227-308` exposes hardware and channel control.
- `ez-daemon/src/web/mod.rs:46-59` has no origin or CORS/CSRF protection. Browser WebSocket handshakes are not governed by CORS, so any page that can reach a non-loopback listener can open `/ws/control` and issue commands as the user.

Impact: a network-adjacent attacker can retune hardware, start recordings, delete channels, and read telemetry/control events. Treat as High (Critical for deployments binding a LAN/WAN address).

## SEC-2 (High): unlimited web connection/subscriber/thread exhaustion

- TCP has a 64-connection semaphore at `ez-daemon/src/server.rs:55-76`; the web server has no corresponding global or per-IP connection limit (`ez-daemon/src/web/mod.rs:37-63`).
- Every successful `/ws/stream/{kind}/{id}` request registers a fresh subscription before upgrade (`ez-daemon/src/web/ws.rs:61-75`), then starts one OS forwarder thread (`:85-106`, calling `spawn_forward_thread` in `ez-daemon/src/server.rs:353-365`).
- There is no bound on the number of simultaneous stream or control WebSockets. Opening many sockets to one existing channel therefore creates unbounded broadcaster registry entries, mpsc queues (capacity 64 each), async writer tasks, and OS threads. `spawn_forward_thread(...).expect(...)` can panic its request task when OS thread creation fails.

Impact: unauthenticated remote DoS via connection/thread/memory exhaustion, even though virtual channel creation itself is capped at 32.

## SEC-3 (Medium): retune API bypasses channel geometry validation

- Creation validates zero bandwidth, ADS-B 2.4 MHz, wideband rate, and passband edges in `ez-daemon/src/state.rs:138-169`.
- `DaemonState::retune` at `ez-daemon/src/state.rs:456-479` does not call `validate_spec`; it forwards arbitrary `bandwidth_hz` and offset to `Channelizer::retune_channel`, which only checks nonzero output rate and center offset (`ez-daemon/src/channelizer.rs:275-306`).
- REST exposes this directly at `ez-daemon/src/web/api.rs:251-257`, and TCP/WS use the same method through `apply_command`.

Impact: a client can retune a channel to a bandwidth greater than the capture rate or with passband edges outside Nyquist. The channelizer then uses decimation 1/full-rate output while metadata advertises the invalid bandwidth, causing aliasing and incorrect downstream assumptions. Extreme `i64::MIN` offsets can also overflow at `state.rs:472` in debug builds.

## SEC-4 (Medium): hardware frequency state can lie and RTL-SDR frequency silently truncates

- REST `set_frequency` always returns 204 and discards errors (`ez-daemon/src/web/api.rs:146-151`); `DaemonState::set_frequency` immediately updates the channelizer and queues the hardware request (`ez-daemon/src/state.rs:127-130`).
- RTL-SDR stores the requested `u64` before invoking a `u32` FFI call (`ez-daemon/src/hardware/rtlsdr.rs:228-237`), so values above `u32::MAX` silently wrap at the ABI boundary; cached status still reports the original value. HackRF/Soapy similarly cache before FFI result (`hackrf.rs:240-248`, `soapy.rs:320-328`).
- Ingest can replace an error snapshot with a success snapshot whenever any command was processed (`ez-daemon/src/ingest.rs:106-138`), so a failed hardware command can be quickly hidden.

Impact: remote clients receive successful responses and apparently valid status while hardware may remain at its prior frequency or be tuned to a wrapped value; channelizer and hardware can disagree.

## SEC-5 (Low/Medium): internal filesystem path disclosure

- `ApiError` serializes raw error strings (`ez-daemon/src/web/api.rs:69-84`), including filesystem contexts from recording operations.
- Recording status always returns the absolute/derived path (`ez-daemon/src/recording.rs:65-72`), and unauthenticated `GET /api/channels/{id}/recording` and `GET /api/recordings` expose it (`ez-daemon/src/web/api.rs:237-249`).

Impact: a remote caller can learn recording directory layout and other local path details. This also assists follow-on attacks if the web listener is exposed.

## SEC-6 (Medium): frontend accepts unvalidated WebSocket payloads and can be crashed/DOM-DoS'd

- `ez-web/src/ui/aircraft-panel.ts:65-91` casts `JSON.parse` directly to `AircraftTelemetry[]`, then assumes `.length`, `.sort`, `ac.icao.toString`, and numeric `.toFixed` methods. A syntactically valid but malformed frame (object, null fields, wrong types, or a huge array) throws in the event callback or causes expensive sort/`innerHTML` DOM replacement.
- `ez-web/src/api/wire.ts:9-72` checks truncation but does not enforce sane maxima, exact frame lengths, finite values, or telemetry `width * height == pixel_count`. It allocates typed-array copies based on attacker-controlled counts up to the WebSocket layer's 64 MiB message default.

Impact: malformed/malicious stream data can terminate UI update handling or consume substantial browser CPU/memory. Callsign HTML escaping is now present at `aircraft-panel.ts:81`, so this finding is payload robustness/DoS rather than the previously reported callsign XSS.

## SEC-7 (Medium): audio monitor async race resurrects a stopped context

- `ez-web/src/audio/monitor.ts:26-31` calls `stop()` then starts a new socket; frames can concurrently trigger `initAudio` at `:33-41`.
- `initAudio` assigns `ctx/node/gain` only after asynchronous `addModule`/`resume` operations (`:43-67`). `stop()` clears these fields and closes the current context at `:75-85`, but has no generation/cancellation token. An in-flight initialization from the old channel can complete afterward and repopulate `this.ctx/node/gain` for a monitor that was stopped or switched.

Impact: stale audio can play after stop/channel switch, contexts/nodes can leak, and rapid user churn can produce inconsistent monitor state.

