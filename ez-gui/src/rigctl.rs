//! Small loopback-only Hamlib/rigctld compatible control server.
//!
//! The server deliberately implements the handful of commands that are useful
//! for a desktop receiver (`f/F`, `m/M`, `v/V`, and `q`).  Socket handling is
//! kept off the egui thread; requests are handed to `CentralApp` so the shared
//! radio state remains the single source of truth.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU16, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc,
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
        }
    }

    pub fn bound_port(&self) -> u16 {
        self.bound_port.load(Ordering::Acquire)
    }

    pub fn set_enabled(&mut self, enabled: bool, port: u16) {
        self.enabled = enabled;
        self.port = port;
        self.stop();
        if enabled {
            self.start();
        }
    }

    pub fn start(&mut self) {
        if !self.enabled || self.join.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let bound_port = Arc::clone(&self.bound_port);
        let port = self.port;
        let tx_worker = tx.clone();
        let join = thread::spawn(move || {
            let listener = match TcpListener::bind(("127.0.0.1", port)) {
                Ok(listener) => listener,
                Err(error) => {
                    eprintln!("[rigctl] bind failed on 127.0.0.1:{port}: {error}");
                    return;
                }
            };
            let _ = listener.set_nonblocking(true);
            if let Ok(address) = listener.local_addr() {
                bound_port.store(address.port(), Ordering::Release);
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
                        break;
                    }
                }
            }
            bound_port.store(0, Ordering::Release);
        });
        self.tx = Some(tx);
        self.rx = Some(rx);
        self.stop = Some(stop);
        self.join = Some(join);
    }

    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Release);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        self.tx = None;
        self.rx = None;
        self.bound_port.store(0, Ordering::Release);
    }

    pub fn poll_requests(&mut self) -> Vec<RigctlRequest> {
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
    let reader_stream = match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    };
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    let mut line = String::new();

    while !stop.load(Ordering::Acquire) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let command = line.trim();
                if command.eq_ignore_ascii_case("q") {
                    break;
                }
                let Some(request) = parse_command(command) else {
                    let _ = writer.write_all(b"RPRT -1\n");
                    continue;
                };
                let (request, reply) = request;
                if tx.send(request).is_err() {
                    let _ = writer.write_all(b"RPRT -11\n");
                    break;
                }
                let response = wait_for_reply(&reply, stop);
                if writer.write_all(response.as_bytes()).is_err() {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
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
