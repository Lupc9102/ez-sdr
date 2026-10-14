//! Small loopback-only Hamlib/rigctld compatible control server.
//!
//! The server deliberately implements the handful of commands that are useful
//! for a desktop receiver (`f/F`, `m/M`, `v/V`, and `q`).  Socket handling is
//! kept off the egui thread; requests are handed to `CentralApp` so the shared
//! radio state remains the single source of truth.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU16, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_millis(750);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// A command waiting for the egui/app thread to apply it.
pub enum RigctlRequest {
    GetFrequency {
        reply: Sender<String>,
    },
    SetFrequency {
        hz: u64,
        reply: Sender<String>,
    },
    GetMode {
        reply: Sender<String>,
    },
    SetMode {
        mode: String,
        bandwidth_hz: Option<u32>,
        reply: Sender<String>,
    },
    GetVolume {
        reply: Sender<String>,
    },
    SetVolume {
        percent: f32,
        reply: Sender<String>,
    },
}

pub struct RigctlServer {
    pub enabled: bool,
    pub port: u16,
    bound_port: Arc<AtomicU16>,
    tx: Option<Sender<RigctlRequest>>,
    rx: Option<Receiver<RigctlRequest>>,
    stop: Option<Arc<AtomicBool>>,
    join: Option<thread::JoinHandle<()>>,
    retired: Vec<(u64, thread::JoinHandle<()>)>,
    status: Arc<Mutex<crate::web_remote::ListenerSnapshot>>,
    generation: u64,
    worker_generation: Arc<std::sync::atomic::AtomicU64>,
}

impl Default for RigctlServer {
    fn default() -> Self {
        Self::new()
    }
}

impl RigctlServer {
    pub fn new() -> Self {
        Self {
            enabled: false,
            port: 4532,
            bound_port: Arc::new(AtomicU16::new(0)),
            tx: None,
            rx: None,
            stop: None,
            join: None,
            retired: Vec::new(),
            status: Arc::new(Mutex::new(crate::web_remote::ListenerSnapshot::default())),
            generation: 0,
            worker_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    pub fn bound_port(&self) -> u16 {
        self.bound_port.load(Ordering::Acquire)
    }

    pub fn set_enabled(&mut self, enabled: bool, port: u16) {
        self.reap_finished();
        let unchanged = self.enabled == enabled && self.port == port;
        if unchanged && (!enabled || self.current_worker_healthy()) {
            return;
        }
        self.enabled = enabled;
        self.port = port;
        let retired_generation = self.generation;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.worker_generation.store(generation, Ordering::Release);
        self.retire_active(retired_generation);
        if enabled {
            self.set_state(generation, crate::web_remote::ListenerState::Starting);
            self.start_generation(generation);
        } else {
            let state = if self.retired.is_empty() {
                crate::web_remote::ListenerState::Stopped
            } else {
                crate::web_remote::ListenerState::Stopping
            };
            self.set_state(generation, state);
        }
    }

    pub fn start(&mut self) {
        if self.enabled {
            self.set_enabled(true, self.port);
        }
    }

    fn current_worker_healthy(&self) -> bool {
        let worker_alive = self.join.as_ref().is_some_and(|join| !join.is_finished());
        let state = self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .state
            .clone();
        worker_alive
            && matches!(
                state,
                crate::web_remote::ListenerState::Starting
                    | crate::web_remote::ListenerState::Listening(_)
            )
    }

    fn set_state(&self, generation: u64, state: crate::web_remote::ListenerState) {
        let mut snapshot = self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot.generation = generation;
        snapshot.state = state;
    }

    fn retire_active(&mut self, retired_generation: u64) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Release);
        }
        self.tx = None;
        self.rx = None;
        if let Some(join) = self.join.take() {
            if join.is_finished() {
                let _ = join.join();
                self.bound_port.store(0, Ordering::Release);
            } else {
                self.retired.push((retired_generation, join));
            }
        }
    }

    fn reap_finished(&mut self) {
        let mut pending = Vec::with_capacity(self.retired.len());
        let mut retired_panicked = false;
        for (retired_generation, handle) in self.retired.drain(..) {
            if handle.is_finished() {
                retired_panicked |= handle.join().is_err();
                let _ = retired_generation;
            } else {
                pending.push((retired_generation, handle));
            }
        }
        self.retired = pending;
        if self.retired.is_empty() {
            let mut snapshot = self
                .status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if snapshot.generation == self.generation
                && snapshot.state == crate::web_remote::ListenerState::Stopping
            {
                self.bound_port.store(0, Ordering::Release);
                snapshot.state = if retired_panicked {
                    crate::web_remote::ListenerState::Failed(
                        "listener worker stopped unexpectedly".into(),
                    )
                } else {
                    crate::web_remote::ListenerState::Stopped
                };
            }
        }
        if self
            .join
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
        {
            let handle = self.join.take().expect("finished rigctl worker handle");
            if handle.join().is_err() {
                let state = self
                    .status
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .state
                    .clone();
                if matches!(
                    state,
                    crate::web_remote::ListenerState::Starting
                        | crate::web_remote::ListenerState::Listening(_)
                ) {
                    self.bound_port.store(0, Ordering::Release);
                    self.set_state(
                        self.generation,
                        crate::web_remote::ListenerState::Failed(
                            "listener worker stopped unexpectedly".into(),
                        ),
                    );
                }
            }
        }
    }

    pub fn status_snapshot(&mut self) -> crate::web_remote::ListenerSnapshot {
        self.reap_finished();
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn start_generation(&mut self, generation: u64) {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let bound_port = Arc::clone(&self.bound_port);
        let worker_generation = Arc::clone(&self.worker_generation);
        let status = Arc::clone(&self.status);
        let port = self.port;
        let tx_worker = tx.clone();
        let join = thread::Builder::new()
            .name("ez-sdr-rigctl".into())
            .spawn(move || {
                let listener = match TcpListener::bind(("127.0.0.1", port)) {
                    Ok(listener) => listener,
                    Err(error) => {
                        if worker_generation.load(Ordering::Acquire) == generation {
                            bound_port.store(0, Ordering::Release);
                        }
                        crate::web_remote::publish_listener_state(
                            &status,
                            generation,
                            crate::web_remote::ListenerState::Failed(format!(
                                "127.0.0.1:{port}: {error}"
                            )),
                        );
                        return;
                    }
                };
                let _ = listener.set_nonblocking(true);
                if let Ok(address) = listener.local_addr() {
                    if worker_generation.load(Ordering::Acquire) == generation {
                        bound_port.store(address.port(), Ordering::Release);
                    }
                    crate::web_remote::publish_listener_state(
                        &status,
                        generation,
                        crate::web_remote::ListenerState::Listening(address.to_string()),
                    );
                    println!("[rigctl] listening on 127.0.0.1:{}", address.port());
                }

                while !stop_worker.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => handle_connection(stream, &tx_worker, &stop_worker),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(POLL_INTERVAL);
                        }
                        Err(error) => {
                            eprintln!("[rigctl] accept failed: {error}");
                            crate::web_remote::publish_listener_state(
                                &status,
                                generation,
                                crate::web_remote::ListenerState::Failed(format!(
                                    "accept failed: {error}"
                                )),
                            );
                            break;
                        }
                    }
                }
                if stop_worker.load(Ordering::Acquire) {
                    let stopping_current_generation = status
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .state
                        == crate::web_remote::ListenerState::Stopping;
                    if worker_generation.load(Ordering::Acquire) == generation
                        || stopping_current_generation
                    {
                        bound_port.store(0, Ordering::Release);
                    }
                    crate::web_remote::publish_listener_state(
                        &status,
                        generation,
                        crate::web_remote::ListenerState::Stopped,
                    );
                } else if worker_generation.load(Ordering::Acquire) == generation {
                    bound_port.store(0, Ordering::Release);
                }
            });
        match join {
            Ok(join) => {
                self.tx = Some(tx);
                self.rx = Some(rx);
                self.stop = Some(stop);
                self.join = Some(join);
            }
            Err(error) => self.set_state(
                generation,
                crate::web_remote::ListenerState::Failed(format!("worker spawn failed: {error}")),
            ),
        }
    }

    pub fn stop(&mut self) {
        let retired_generation = self.generation;
        self.generation = self.generation.wrapping_add(1);
        self.worker_generation
            .store(self.generation, Ordering::Release);
        let generation = self.generation;
        self.retire_active(retired_generation);
        let state = if self.retired.is_empty() {
            crate::web_remote::ListenerState::Stopped
        } else {
            crate::web_remote::ListenerState::Stopping
        };
        self.set_state(generation, state);
        self.reap_finished();
    }

    pub fn poll_requests(&mut self) -> Vec<RigctlRequest> {
        self.reap_finished();
        let mut requests = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(request) = rx.try_recv() {
                requests.push(request);
            }
        }
        requests
    }
}

impl Drop for RigctlServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle_connection(stream: TcpStream, tx: &Sender<RigctlRequest>, stop: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(POLL_INTERVAL));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let mut stream = stream;
    let mut line = Vec::new();
    let mut discarding = false;
    let mut buffer = [0; 512];
    const MAX_COMMAND_BYTES: usize = 1024;

    while !stop.load(Ordering::Acquire) {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                for &byte in &buffer[..count] {
                    if byte != b'\n' {
                        if !discarding {
                            if line.len() >= MAX_COMMAND_BYTES {
                                line.clear();
                                discarding = true;
                            } else {
                                line.push(byte);
                            }
                        }
                        continue;
                    }
                    if discarding {
                        discarding = false;
                        if stream.write_all(b"RPRT -1\n").is_err() {
                            return;
                        }
                        continue;
                    }
                    let command = std::mem::take(&mut line);
                    let command = std::str::from_utf8(&command).unwrap_or("").trim();
                    if command.eq_ignore_ascii_case("q") {
                        return;
                    }
                    let Some((request, reply)) = parse_command(command) else {
                        if stream.write_all(b"RPRT -1\n").is_err() {
                            return;
                        }
                        continue;
                    };
                    if tx.send(request).is_err() {
                        let _ = stream.write_all(b"RPRT -11\n");
                        return;
                    }
                    let response = wait_for_reply(&reply, stop);
                    if stream.write_all(response.as_bytes()).is_err() {
                        return;
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // Retain the incomplete line across arbitrary TCP fragmentation.
            }
            Err(_) => break,
        }
    }
}

fn wait_for_reply(reply: &Receiver<String>, stop: &AtomicBool) -> String {
    let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return "RPRT -5\n".to_string();
        }
        let remaining = (deadline - now).min(POLL_INTERVAL);
        match reply.recv_timeout(remaining) {
            Ok(response) => return response,
            Err(mpsc::RecvTimeoutError::Timeout) if stop.load(Ordering::Acquire) => {
                return "RPRT -11\n".to_string();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return "RPRT -11\n".to_string(),
        }
    }
}

type ParsedRequest = (RigctlRequest, ReceiverReply);
type ReceiverReply = Receiver<String>;

fn parse_command(command: &str) -> Option<ParsedRequest> {
    let mut fields = command.split_whitespace();
    let op = fields.next()?;
    let (reply_tx, reply_rx) = mpsc::channel();
    let request = match op {
        "f" => RigctlRequest::GetFrequency { reply: reply_tx },
        "F" => {
            let hz = fields.next()?.parse::<u64>().ok()?;
            RigctlRequest::SetFrequency {
                hz,
                reply: reply_tx,
            }
        }
        "m" => RigctlRequest::GetMode { reply: reply_tx },
        "M" => {
            let mode = fields.next()?.to_ascii_uppercase();
            let bandwidth_hz = fields.next().and_then(|value| value.parse().ok());
            RigctlRequest::SetMode {
                mode,
                bandwidth_hz,
                reply: reply_tx,
            }
        }
        "v" => RigctlRequest::GetVolume { reply: reply_tx },
        "V" => {
            let percent = fields.next()?.parse::<f32>().ok()?;
            if !percent.is_finite() {
                return None;
            }
            RigctlRequest::SetVolume {
                percent: percent.clamp(0.0, 100.0),
                reply: reply_tx,
            }
        }
        _ => return None,
    };
    Some((request, reply_rx))
}

pub fn ok_response() -> String {
    "RPRT 0\n".to_string()
}

pub fn error_response(code: i32) -> String {
    format!("RPRT {code}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn every_tcp_fragment_boundary_preserves_commands_across_timeouts() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, &tx, &worker_stop);
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.set_nodelay(true).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut replies = BufReader::new(client.try_clone().unwrap());
        for command in [
            "F 1090000000\n",
            "M AM 12000\n",
            "V 50\n",
            "f\n",
            "m\n",
            "v\n",
        ] {
            for split in 1..command.len() {
                client.write_all(&command.as_bytes()[..split]).unwrap();
                thread::sleep(POLL_INTERVAL * 2);
                assert!(
                    rx.try_recv().is_err(),
                    "partial command must not be executed"
                );
                client.write_all(&command.as_bytes()[split..]).unwrap();
                let request = rx.recv_timeout(Duration::from_secs(1)).unwrap();
                let reply = match request {
                    RigctlRequest::SetFrequency { hz, reply } => {
                        assert_eq!(command, "F 1090000000\n");
                        assert_eq!(hz, 1_090_000_000);
                        reply
                    }
                    RigctlRequest::SetMode {
                        mode,
                        bandwidth_hz,
                        reply,
                    } => {
                        assert_eq!(command, "M AM 12000\n");
                        assert_eq!(mode, "AM");
                        assert_eq!(bandwidth_hz, Some(12_000));
                        reply
                    }
                    RigctlRequest::SetVolume { percent, reply } => {
                        assert_eq!(command, "V 50\n");
                        assert_eq!(percent, 50.0);
                        reply
                    }
                    RigctlRequest::GetFrequency { reply } => {
                        assert_eq!(command, "f\n");
                        reply
                    }
                    RigctlRequest::GetMode { reply } => {
                        assert_eq!(command, "m\n");
                        reply
                    }
                    RigctlRequest::GetVolume { reply } => {
                        assert_eq!(command, "v\n");
                        reply
                    }
                };
                reply.send("RPRT 0\n".into()).unwrap();
                let mut response = String::new();
                replies.read_line(&mut response).unwrap();
                assert_eq!(response, "RPRT 0\n");
                assert!(rx.try_recv().is_err());
            }
        }
        client.write_all(&vec![b'X'; 2048]).unwrap();
        client.write_all(b"\nf\n").unwrap();
        let RigctlRequest::GetFrequency { reply } =
            rx.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("oversized line must be discarded");
        };
        reply.send("118700000\n".into()).unwrap();
        let mut response = String::new();
        replies.read_line(&mut response).unwrap();
        assert_eq!(response, "RPRT -1\n");
        response.clear();
        replies.read_line(&mut response).unwrap();
        assert_eq!(response, "118700000\n");
        client.write_all(b"F 1").unwrap();
        let stopping = std::time::Instant::now();
        stop.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(stopping.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn parses_getters_and_setters() {
        assert!(matches!(
            parse_command("f"),
            Some((RigctlRequest::GetFrequency { .. }, _))
        ));
        assert!(matches!(
            parse_command("F 118700000"),
            Some((RigctlRequest::SetFrequency { hz: 118700000, .. }, _))
        ));
        assert!(matches!(
            parse_command("m"),
            Some((RigctlRequest::GetMode { .. }, _))
        ));
        assert!(matches!(
            parse_command("M NFM 12500"),
            Some((RigctlRequest::SetMode { ref mode, bandwidth_hz: Some(12500), .. }, _)) if mode == "NFM"
        ));
        assert!(matches!(
            parse_command("V 72.5"),
            Some((RigctlRequest::SetVolume { percent, .. }, _)) if (percent - 72.5).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn rejects_malformed_and_unknown_commands() {
        assert!(parse_command("").is_none());
        assert!(parse_command("F nope").is_none());
        assert!(parse_command("V nan").is_none());
        assert!(parse_command("z").is_none());
    }

    #[test]
    fn volume_is_bounded() {
        let (request, _) = parse_command("V 500").expect("volume command");
        assert!(matches!(request, RigctlRequest::SetVolume { percent, .. } if percent == 100.0));
    }

    fn wait_for_state(
        server: &mut RigctlServer,
        predicate: impl Fn(&crate::web_remote::ListenerState) -> bool,
    ) -> crate::web_remote::ListenerSnapshot {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = server.status_snapshot();
            if predicate(&snapshot.state) {
                return snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "listener state timed out: {snapshot:?}"
            );
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn port_is_released(port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    fn wait_for_port_release(port: u16) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !port_is_released(port) && std::time::Instant::now() < deadline {
            thread::sleep(POLL_INTERVAL);
        }
        assert!(port_is_released(port), "port {port} should be released");
    }

    #[test]
    fn occupied_port_reports_failure_and_same_config_retries() {
        let blocker = TcpListener::bind("127.0.0.1:0").expect("occupy loopback port");
        let port = blocker.local_addr().unwrap().port();
        let mut server = RigctlServer::new();
        server.set_enabled(true, port);
        let failed = wait_for_state(&mut server, |state| {
            matches!(state, crate::web_remote::ListenerState::Failed(_))
        });
        assert!(failed.state.to_string().contains(&port.to_string()));

        drop(blocker);
        server.set_enabled(true, port);
        let listening = wait_for_state(&mut server, |state| {
            matches!(state, crate::web_remote::ListenerState::Listening(_))
        });
        assert!(listening.state.to_string().contains(&port.to_string()));
        assert_eq!(server.bound_port(), port);

        server.set_enabled(false, port);
        wait_for_state(&mut server, |state| {
            *state == crate::web_remote::ListenerState::Stopped
        });
        wait_for_port_release(port);
    }

    #[test]
    fn unchanged_config_keeps_client_and_changed_port_releases_old_listener() {
        let mut server = RigctlServer::new();
        server.set_enabled(true, 0);
        wait_for_state(&mut server, |state| {
            matches!(state, crate::web_remote::ListenerState::Listening(_))
        });
        let first_port = server.bound_port();
        let generation = server.status_snapshot().generation;
        let mut client =
            TcpStream::connect(("127.0.0.1", first_port)).expect("connect rigctl client");
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();

        server.set_enabled(true, 0);
        assert_eq!(
            server.status_snapshot().generation,
            generation,
            "healthy no-op must retain worker generation"
        );
        client
            .write_all(b"z\n")
            .expect("send command over retained client");
        let mut response = String::new();
        BufReader::new(client.try_clone().unwrap())
            .read_line(&mut response)
            .unwrap();
        assert_eq!(response, "RPRT -1\n");

        let replacement = TcpListener::bind("127.0.0.1:0").unwrap();
        let new_port = replacement.local_addr().unwrap().port();
        drop(replacement);
        server.set_enabled(true, new_port);
        wait_for_state(
            &mut server,
            |state| matches!(state, crate::web_remote::ListenerState::Listening(endpoint) if endpoint.ends_with(&new_port.to_string())),
        );
        drop(client);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !port_is_released(first_port) && std::time::Instant::now() < deadline {
            thread::sleep(POLL_INTERVAL);
        }
        assert!(
            port_is_released(first_port),
            "previous rigctl port should be released"
        );
        server.stop();
        wait_for_state(&mut server, |state| {
            *state == crate::web_remote::ListenerState::Stopped
        });
        wait_for_port_release(new_port);
    }

    #[test]
    fn stop_with_idle_client_is_nonblocking_and_releases_listener() {
        let mut server = RigctlServer::new();
        server.set_enabled(true, 0);
        wait_for_state(&mut server, |state| {
            matches!(state, crate::web_remote::ListenerState::Listening(_))
        });
        let port = server.bound_port();
        let mut client =
            TcpStream::connect(("127.0.0.1", port)).expect("connect non-reading client");
        client
            .write_all(b"z\n")
            .expect("send command without reading reply");
        thread::sleep(POLL_INTERVAL * 2);
        let start = std::time::Instant::now();
        server.stop();
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "stop must not join on caller thread"
        );
        wait_for_state(&mut server, |state| {
            *state == crate::web_remote::ListenerState::Stopped
        });
        drop(client);
        wait_for_port_release(port);
    }

    #[test]
    fn repeated_enable_disable_releases_each_listener() {
        let mut server = RigctlServer::new();
        for _ in 0..4 {
            server.set_enabled(true, 0);
            wait_for_state(&mut server, |state| {
                matches!(state, crate::web_remote::ListenerState::Listening(_))
            });
            let port = server.bound_port();
            assert_ne!(port, 0);
            server.set_enabled(false, 0);
            wait_for_state(&mut server, |state| {
                *state == crate::web_remote::ListenerState::Stopped
            });
            wait_for_port_release(port);
        }
    }

    #[test]
    #[ignore = "requires loopback socket permission"]
    fn lifecycle_can_toggle_without_leaking() {
        let mut server = RigctlServer::new();
        server.set_enabled(true, 0);
        for _ in 0..20 {
            if server.bound_port() != 0 {
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }
        assert_ne!(server.bound_port(), 0);
        server.set_enabled(false, 0);
        assert_eq!(server.bound_port(), 0);
        assert!(server.poll_requests().is_empty());
    }

    #[test]
    #[ignore = "requires loopback socket permission"]
    fn loopback_round_trip_uses_app_owned_response() {
        let mut server = RigctlServer::new();
        server.set_enabled(true, 0);
        for _ in 0..30 {
            if server.bound_port() != 0 {
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }
        let port = server.bound_port();
        assert_ne!(port, 0);

        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("rigctl listener");
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout");
        stream.write_all(b"f\n").expect("write frequency query");
        let request = loop {
            let mut requests = server.poll_requests();
            if let Some(request) = requests.pop() {
                break request;
            }
            thread::sleep(POLL_INTERVAL);
        };
        let RigctlRequest::GetFrequency { reply } = request else {
            panic!("expected frequency query");
        };
        reply.send("118700000\n".into()).expect("send response");
        let mut response = String::new();
        BufReader::new(stream.try_clone().expect("clone stream"))
            .read_line(&mut response)
            .expect("read frequency response");
        assert_eq!(response, "118700000\n");
        stream.write_all(b"q\n").expect("quit command");
        server.stop();
    }
}
