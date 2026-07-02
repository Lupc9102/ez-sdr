//! Network I/O (Beast/SBS/raw) - translated from net_io.c

use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::sync::{Arc, Mutex};
use std::thread;

/// A connected client.
struct Client {
    stream: TcpStream,
    #[allow(dead_code)]
    addr: SocketAddr,
}

/// Network output server supporting Beast, SBS, and raw AVR formats.
pub struct NetIo {
    beast_clients: Arc<Mutex<Vec<Client>>>,
    sbs_clients: Arc<Mutex<Vec<Client>>>,
    raw_clients: Arc<Mutex<Vec<Client>>>,
}

impl Default for NetIo {
    fn default() -> Self {
        Self::new()
    }
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
        let escaped = encode_beast_frame(timestamp, signal, msg);
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

/// Build a framed, escaped Beast-format message: 0x1a marker (never escaped)
/// + type indicator + 6-byte big-endian timestamp + signal + message bytes,
/// with any 0x1a byte in that payload doubled per the Beast escaping rule.
fn encode_beast_frame(timestamp: u64, signal: u8, msg: &[u8]) -> Vec<u8> {
    // Beast type indicator: '1' (Mode-AC, 2 bytes), '2' (Mode S short, 7 bytes),
    // '3' (Mode S long, 14 bytes).
    let indicator = match msg.len() {
        2 => 0x31,
        7 => 0x32,
        _ => 0x33,
    };
    let mut payload = Vec::with_capacity(msg.len() + 7);
    payload.push(indicator);
    payload.extend_from_slice(&timestamp.to_be_bytes()[2..]); // 6 bytes big-endian
    payload.push(signal);
    payload.extend_from_slice(msg);

    let mut escaped = Vec::with_capacity(payload.len() * 2 + 1);
    escaped.push(0x1a);
    for &b in &payload {
        escaped.push(b);
        if b == 0x1a {
            escaped.push(0x1a);
        }
    }
    escaped
}

fn hex_digit(n: u8) -> u8 {
    match n {
        0..=9 => b'0' + n,
        10..=15 => b'A' + (n - 10),
        _ => b'?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beast_frame_starts_with_unescaped_marker() {
        let msg = [0u8; 7]; // short Mode S, no 0x1a bytes
        let frame = encode_beast_frame(0, 0, &msg);
        // The very first byte must be the lone frame marker, not doubled.
        assert_eq!(frame[0], 0x1a);
        assert_ne!(frame[1], 0x1a);
    }

    #[test]
    fn beast_frame_picks_indicator_by_length() {
        assert_eq!(encode_beast_frame(0, 0, &[0u8; 2])[1], 0x31); // Mode-AC
        assert_eq!(encode_beast_frame(0, 0, &[0u8; 7])[1], 0x32); // short Mode S
        assert_eq!(encode_beast_frame(0, 0, &[0u8; 14])[1], 0x33); // long Mode S
    }

    #[test]
    fn beast_frame_escapes_0x1a_in_payload_only() {
        // Craft a message whose bytes contain 0x1a; the marker at index 0
        // must stay a single byte while the in-payload occurrence is doubled.
        let msg = [0x1a, 1, 2, 3, 4, 5, 6];
        let frame = encode_beast_frame(0, 0, &msg);
        assert_eq!(frame[0], 0x1a);
        assert_ne!(frame[1], 0x1a); // indicator byte, not part of the escape run
        // Count total 0x1a occurrences: 1 marker + 2 (escaped payload byte) = 3.
        let count = frame.iter().filter(|&&b| b == 0x1a).count();
        assert_eq!(count, 3);
    }
}
