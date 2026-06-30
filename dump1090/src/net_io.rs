//! Network I/O (Beast/SBS/raw) - translated from net_io.c

use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::sync::{Arc, Mutex};
use std::thread;

/// A connected client.
struct Client {
    stream: TcpStream,
    addr: SocketAddr,
}

/// Network output server supporting Beast, SBS, and raw AVR formats.
pub struct NetIo {
    beast_clients: Arc<Mutex<Vec<Client>>>,
    sbs_clients: Arc<Mutex<Vec<Client>>>,
    raw_clients: Arc<Mutex<Vec<Client>>>,
}

impl NetIo {
    pub fn new() -> Self {
        NetIo {
            beast_clients: Arc::new(Mutex::new(Vec::new())),
            sbs_clients: Arc::new(Mutex::new(Vec::new())),
            raw_clients: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Start listening on the given ports.
    pub fn start(
        &self,
        beast_port: u16,
        sbs_port: u16,
        raw_port: u16,
    ) -> anyhow::Result<()> {
        Self::spawn_listener(beast_port, self.beast_clients.clone(), "Beast");
        Self::spawn_listener(sbs_port, self.sbs_clients.clone(), "SBS");
        Self::spawn_listener(raw_port, self.raw_clients.clone(), "raw AVR");
        Ok(())
    }

    fn spawn_listener(port: u16, clients: Arc<Mutex<Vec<Client>>>, name: &'static str) {
        thread::spawn(move || {
            let listener = match TcpListener::bind(("0.0.0.0", port)) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("net_io: failed to bind {} port {}: {}", name, port, e);
                    return;
                }
            };
            eprintln!("net_io: {} output on port {}", name, port);
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if let Err(e) = stream.set_nonblocking(true) {
                            eprintln!("net_io: failed to set non-blocking: {}", e);
                            continue;
                        }
                        if let Ok(addr) = stream.peer_addr() {
                            eprintln!("net_io: {} client connected from {}", name, addr);
                            if let Ok(mut vec) = clients.lock() {
                                vec.push(Client { stream, addr });
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("net_io: {} accept error: {}", name, e);
                    }
                }
            }
        });
    }

    fn prune_and_send(clients: &Arc<Mutex<Vec<Client>>>, data: &[u8]) {
        let mut vec = match clients.lock() {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut i = 0;
        while i < vec.len() {
            match vec[i].stream.write(data) {
                Ok(_) => i += 1,
                Err(e) if e.kind() == ErrorKind::WouldBlock => i += 1,
                Err(_) => {
                    vec.swap_remove(i);
                }
            }
        }
    }

    /// Send a Mode S message in Beast format.
    pub fn send_beast(&self, timestamp: u64, signal: u8, msg: &[u8]) {
        let mut out = Vec::with_capacity(msg.len() + 9);
        out.push(0x1a);
        out.push(0x31); // Mode S long/short indicator (simplified)
        out.extend_from_slice(&timestamp.to_be_bytes()[2..]); // 6 bytes big-endian
        out.push(signal);
        out.extend_from_slice(msg);
        // Escape 0x1a in payload
        let mut escaped = Vec::with_capacity(out.len() + 8);
        for &b in &out {
            escaped.push(b);
            if b == 0x1a {
                escaped.push(0x1a);
            }
        }
        Self::prune_and_send(&self.beast_clients, &escaped);
    }

    /// Send a line in SBS (BaseStation) format.
    pub fn send_sbs(&self, line: &str) {
        let mut out = line.as_bytes().to_vec();
        out.push(b'\r');
        out.push(b'\n');
        Self::prune_and_send(&self.sbs_clients, &out);
    }

    /// Send a message in raw AVR format.
    pub fn send_raw(&self, msg: &[u8], downlink: bool) {
        let prefix = if downlink { '*' } else { '@' };
        let mut out = Vec::with_capacity(msg.len() * 2 + 4);
        out.push(prefix as u8);
        for b in msg {
            out.push(hex_digit(*b >> 4));
            out.push(hex_digit(*b & 0x0f));
        }
        out.push(b';');
        out.push(b'\r');
        out.push(b'\n');
        Self::prune_and_send(&self.raw_clients, &out);
    }
}

fn hex_digit(n: u8) -> u8 {
    match n {
        0..=9 => b'0' + n,
        10..=15 => b'A' + (n - 10),
        _ => b'?',
    }
}
