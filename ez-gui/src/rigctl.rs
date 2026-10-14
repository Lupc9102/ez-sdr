//! Small loopback-only Hamlib/rigctld compatible control server.
//!
//! The server deliberately implements the handful of commands that are useful
//! for a desktop receiver (`f/F`, `m/M`, `v/V`, `l/L`, and `q`). Socket handling is
//! kept off the egui thread; requests are handed to `CentralApp` so the shared
//! radio state remains the single source of truth.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU16, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_millis(750);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_CLIENTS: usize = 8;
const COMMAND_QUEUE_CAPACITY: usize = 64;

/// A command waiting for the egui/app thread to apply it.
pub enum RigctlRequest {
    GetFrequency {
        reply: SyncSender<String>,
    },
    SetFrequency {
        hz: u64,
        reply: SyncSender<String>,
    },
    GetMode {
        reply: SyncSender<String>,
    },
    SetMode {
        mode: String,
        bandwidth_hz: i32,
        reply: SyncSender<String>,
    },
    GetVfo {
        reply: SyncSender<String>,
    },
    SetVfo {
        vfo: String,
        reply: SyncSender<String>,
    },
    GetAudioLevel {
        reply: SyncSender<String>,
    },
    SetAudioLevel {
        level: f32,
        reply: SyncSender<String>,
    },
}

pub struct RigctlServer {
    pub enabled: bool,
    pub port: u16,
    bound_port: Arc<AtomicU16>,
    tx: Option<SyncSender<RigctlRequest>>,
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
        let (tx, rx) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
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

                let mut client_workers = Vec::with_capacity(MAX_CLIENTS);
                let mut accept_failed = false;
                while !stop_worker.load(Ordering::Acquire) {
                    reap_client_workers(&mut client_workers);
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if client_workers.len() >= MAX_CLIENTS {
                                let mut stream = stream;
                                let _ = stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT));
                                let _ = stream.write_all(b"RPRT -11\n");
                                continue;
                            }
                            let mut rejected_stream = stream.try_clone().ok();
                            let tx_client = tx_worker.clone();
                            let stop_client = Arc::clone(&stop_worker);
                            match thread::Builder::new()
                                .name("ez-sdr-rigctl-client".into())
                                .spawn(move || handle_connection(stream, &tx_client, &stop_client))
                            {
                                Ok(worker) => client_workers.push(worker),
                                Err(_) => {
                                    if let Some(stream) = &mut rejected_stream {
                                        let _ = stream.write_all(b"RPRT -11\n");
                                    }
                                }
                            }
                        }
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
                            accept_failed = true;
                            break;
                        }
                    }
                }
                stop_worker.store(true, Ordering::Release);
                for client in client_workers {
                    let _ = client.join();
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
                    if !accept_failed {
                        crate::web_remote::publish_listener_state(
                            &status,
                            generation,
                            crate::web_remote::ListenerState::Stopped,
                        );
                    }
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

fn reap_client_workers(workers: &mut Vec<thread::JoinHandle<()>>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            let worker = workers.swap_remove(index);
            let _ = worker.join();
        } else {
            index += 1;
        }
    }
}

fn handle_connection(stream: TcpStream, tx: &SyncSender<RigctlRequest>, stop: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(POLL_INTERVAL));
    let _ = stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT));
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
                    match tx.try_send(request) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                            let _ = stream.write_all(b"RPRT -11\n");
                            return;
                        }
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

type ParsedRequest = (RigctlRequest, Receiver<String>);

fn parse_command(command: &str) -> Option<ParsedRequest> {
    let mut fields = command.split_whitespace();
    let op = fields.next()?;
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let request = match op {
        "f" if fields.next().is_none() => RigctlRequest::GetFrequency { reply: reply_tx },
        "F" => {
            let hz = fields.next()?.parse::<u64>().ok()?;
            if fields.next().is_some() {
                return None;
            }
            RigctlRequest::SetFrequency {
                hz,
                reply: reply_tx,
            }
        }
        "m" if fields.next().is_none() => RigctlRequest::GetMode { reply: reply_tx },
        "M" => {
            let mode = fields.next()?.to_ascii_uppercase();
            let bandwidth_hz = fields.next()?.parse::<i32>().ok()?;
            if fields.next().is_some() || bandwidth_hz < -1 {
                return None;
            }
            RigctlRequest::SetMode {
                mode,
                bandwidth_hz,
                reply: reply_tx,
            }
        }
        "v" if fields.next().is_none() => RigctlRequest::GetVfo { reply: reply_tx },
        "V" => {
            let vfo = fields.next()?.to_ascii_uppercase();
            if fields.next().is_some() || !matches!(vfo.as_str(), "VFOA" | "CURRVFO") {
                return None;
            }
            RigctlRequest::SetVfo {
                vfo,
                reply: reply_tx,
            }
        }
        "l" => {
            let level = fields.next()?.to_ascii_uppercase();
            if fields.next().is_some() || level != "AF" {
                return None;
            }
            RigctlRequest::GetAudioLevel { reply: reply_tx }
        }
        "L" => {
            let level_name = fields.next()?.to_ascii_uppercase();
            let level = fields.next()?.parse::<f32>().ok()?;
            if fields.next().is_some()
                || level_name != "AF"
                || !level.is_finite()
                || !(0.0..=1.0).contains(&level)
            {
                return None;
            }
            RigctlRequest::SetAudioLevel {
                level,
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
        let (tx, rx) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
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
            "V VFOA\n",
            "f\n",
            "m\n",
            "v\n",
            "L AF 0.5\n",
            "l AF\n",
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
                        assert_eq!(bandwidth_hz, 12_000);
                        reply
                    }
                    RigctlRequest::SetVfo { vfo, reply } => {
                        assert_eq!(command, "V VFOA\n");
                        assert_eq!(vfo, "VFOA");
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
                    RigctlRequest::GetVfo { reply } => {
                        assert_eq!(command, "v\n");
                        reply
                    }
                    RigctlRequest::SetAudioLevel { level, reply } => {
                        assert_eq!(command, "L AF 0.5\n");
                        assert_eq!(level, 0.5);
                        reply
                    }
                    RigctlRequest::GetAudioLevel { reply } => {
                        assert_eq!(command, "l AF\n");
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
            parse_command("M FM 12500"),
            Some((RigctlRequest::SetMode { ref mode, bandwidth_hz: 12500, .. }, _)) if mode == "FM"
        ));
        assert!(parse_command("V VFOB").is_none());
        assert!(matches!(
            parse_command("V VFOA"),
            Some((RigctlRequest::SetVfo { ref vfo, .. }, _)) if vfo == "VFOA"
        ));
        assert!(matches!(
            parse_command("v"),
            Some((RigctlRequest::GetVfo { .. }, _))
        ));
        assert!(matches!(
            parse_command("L AF 0.75"),
            Some((RigctlRequest::SetAudioLevel { level, .. }, _)) if (level - 0.75).abs() < f32::EPSILON
        ));
        assert!(matches!(
            parse_command("l AF"),
            Some((RigctlRequest::GetAudioLevel { .. }, _))
        ));
    }

    #[test]
    fn rejects_malformed_and_unknown_commands() {
        assert!(parse_command("").is_none());
        assert!(parse_command("F nope").is_none());
        assert!(parse_command("L AF nan").is_none());
        assert!(parse_command("L AF -0.01").is_none());
        assert!(parse_command("L AF 1.01").is_none());
        assert!(parse_command("L RF 0.5").is_none());
        assert!(parse_command("l RF").is_none());
        assert!(parse_command("M FM nope").is_none());
        assert!(parse_command("F 100 200").is_none());
        assert!(parse_command("f extra").is_none());
        assert!(parse_command("z").is_none());
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
    fn idle_client_does_not_block_a_second_client_request() {
        let mut server = RigctlServer::new();
        server.set_enabled(true, 0);
        wait_for_state(&mut server, |state| {
            matches!(state, crate::web_remote::ListenerState::Listening(_))
        });
        let port = server.bound_port();
        let idle = TcpStream::connect(("127.0.0.1", port)).expect("first idle rigctl client");
        let mut active = TcpStream::connect(("127.0.0.1", port)).expect("second rigctl client");
        active
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        active.write_all(b"f\n").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let request = loop {
            if let Some(request) = server.poll_requests().into_iter().next() {
                break request;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "second client was starved"
            );
            thread::sleep(POLL_INTERVAL);
        };
        let RigctlRequest::GetFrequency { reply } = request else {
            panic!("frequency request expected from second client");
        };
        reply.send("118700000\n".into()).unwrap();
        let mut response = String::new();
        BufReader::new(active.try_clone().unwrap())
            .read_line(&mut response)
            .unwrap();
        assert_eq!(response, "118700000\n");

        drop(active);
        drop(idle);
        server.stop();
        wait_for_state(&mut server, |state| {
            *state == crate::web_remote::ListenerState::Stopped
        });
        wait_for_port_release(port);
    }

    #[test]
    fn client_cap_and_command_queue_are_bounded_during_stop() {
        let mut server = RigctlServer::new();
        server.set_enabled(true, 0);
        wait_for_state(&mut server, |state| {
            matches!(state, crate::web_remote::ListenerState::Listening(_))
        });
        let port = server.bound_port();
        let mut clients = (0..MAX_CLIENTS)
            .map(|_| {
                let stream = TcpStream::connect(("127.0.0.1", port)).expect("rigctl client");
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                stream
            })
            .collect::<Vec<_>>();
        thread::sleep(POLL_INTERVAL * 6);

        let excess = TcpStream::connect(("127.0.0.1", port)).expect("excess client");
        excess
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut rejected = String::new();
        BufReader::new(excess.try_clone().unwrap())
            .read_line(&mut rejected)
            .unwrap();
        assert_eq!(rejected, "RPRT -11\n");

        let tx = server.tx.as_ref().expect("active request queue").clone();
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            let (reply, _receiver) = mpsc::sync_channel(1);
            assert!(matches!(
                tx.try_send(RigctlRequest::GetFrequency { reply }),
                Ok(())
            ));
        }
        let (reply, _receiver) = mpsc::sync_channel(1);
        assert!(matches!(
            tx.try_send(RigctlRequest::GetFrequency { reply }),
            Err(TrySendError::Full(_))
        ));

        clients[0].write_all(b"f\n").unwrap(); // This client deliberately never reads its reply.
        let started = std::time::Instant::now();
        server.stop();
        assert!(started.elapsed() < Duration::from_millis(100));
        wait_for_state(&mut server, |state| {
            *state == crate::web_remote::ListenerState::Stopped
        });
        drop(tx);
        drop(excess);
        drop(clients);
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
