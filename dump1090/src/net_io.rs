//! Network I/O (Beast/SBS/raw) - translated from `net_io.c`

use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

/// A connected client.
struct Client {
    stream: TcpStream,
}

/// Maximum concurrent clients per output port. Bounds memory (each idle
/// connection holds a socket + kernel buffers) and the per-block send loop,
/// which runs under a single mutex.
const MAX_CLIENTS_PER_PORT: usize = 64;

/// Network output server supporting Beast, SBS, and raw AVR formats.
///
/// Note: like the reference `net_io.c`, the Beast/SBS/raw feeder ports have
/// no authentication — they are plaintext broadcast outputs in the `dump1090`
/// tradition. Bind to loopback or firewall them if the LAN is untrusted.
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
    /// Create a new `NetIo` with empty client lists for Beast, SBS, and raw AVR output.
    #[must_use]
    pub fn new() -> Self {
        NetIo {
            beast_clients: Arc::new(Mutex::new(Vec::new())),
            sbs_clients: Arc::new(Mutex::new(Vec::new())),
            raw_clients: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Start listening on the given ports.
    ///
    /// # Errors
    /// Returns `anyhow::Error` if a TCP listener fails to bind on any of the
    /// requested ports.
    pub fn start(&self, beast_port: u16, sbs_port: u16, raw_port: u16) -> anyhow::Result<()> {
        let beast_listener = TcpListener::bind(("0.0.0.0", beast_port))
            .map_err(|e| anyhow::anyhow!("net_io: failed to bind Beast port {beast_port}: {e}"))?;
        let sbs_listener = TcpListener::bind(("0.0.0.0", sbs_port))
            .map_err(|e| anyhow::anyhow!("net_io: failed to bind SBS port {sbs_port}: {e}"))?;
        let raw_listener = TcpListener::bind(("0.0.0.0", raw_port))
            .map_err(|e| anyhow::anyhow!("net_io: failed to bind raw AVR port {raw_port}: {e}"))?;

        Self::spawn_listener_loop(
            beast_listener,
            self.beast_clients.clone(),
            "Beast",
            beast_port,
        );
        Self::spawn_listener_loop(sbs_listener, self.sbs_clients.clone(), "SBS", sbs_port);
        Self::spawn_listener_loop(raw_listener, self.raw_clients.clone(), "raw AVR", raw_port);
        Ok(())
    }

    fn spawn_listener_loop(
        listener: TcpListener,
        clients: Arc<Mutex<Vec<Client>>>,
        name: &'static str,
        port: u16,
    ) {
        thread::spawn(move || {
            eprintln!("net_io: {name} output on port {port}");
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if let Err(e) = stream.set_nonblocking(true) {
                            eprintln!("net_io: failed to set non-blocking: {e}");
                            continue;
                        }
                        if let Ok(addr) = stream.peer_addr() {
                            if Self::push_client(&clients, stream) {
                                eprintln!("net_io: {name} client connected from {addr}");
                            } else {
                                eprintln!(
                                    "net_io: {name} client from {addr} rejected: client limit ({MAX_CLIENTS_PER_PORT}) reached"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("net_io: {name} accept error: {e}");
                    }
                }
            }
        });
    }

    /// Add a client, enforcing [`MAX_CLIENTS_PER_PORT`]. Returns `false` when
    /// the connection was rejected (the stream is then closed by dropping).
    fn push_client(clients: &Arc<Mutex<Vec<Client>>>, stream: TcpStream) -> bool {
        match clients.lock() {
            Ok(mut vec) => {
                if vec.len() >= MAX_CLIENTS_PER_PORT {
                    return false;
                }
                vec.push(Client { stream });
                true
            }
            Err(_) => false,
        }
    }

    fn prune_and_send(clients: &Arc<Mutex<Vec<Client>>>, data: &[u8]) {
        let mut vec = match clients.lock() {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut i = 0;
        while i < vec.len() {
            match vec[i].stream.write_all(data) {
                Ok(()) => i += 1,
                // A non-blocking socket reports `WouldBlock` when its send
                // buffer is full. Skipping it (the old behavior) silently
                // truncates framed protocols like Beast and desynchronizes
                // downstream decoders, while the lagging client stays
                // connected forever. Drop slow consumers instead: feeders
                // reconnect, and the cap above bounds reconnection churn.
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    vec.swap_remove(i);
                }
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

    /// Send a line in SBS (`BaseStation`) format.
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
///   with any 0x1a byte in that payload doubled per the Beast escaping rule.
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
    // Beast timestamps are 6-byte big-endian; truncate the high 2 bytes of
    // the 8-byte u64. Valid because the protocol defines a 48-bit timestamp.
    payload.extend_from_slice(&timestamp.to_be_bytes()[2..]);
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
    use std::net::TcpListener;
    use std::time::Duration;

    /// Connect `n` loopback clients to `addr` (the listener backlog accepts
    /// them without an accept thread) and return the connected streams.
    fn connect_clients(addr: &std::net::SocketAddr, n: usize) -> Vec<TcpStream> {
        (0..n)
            .map(|_| TcpStream::connect(addr).expect("loopback connect should succeed"))
            .collect()
    }

    #[test]
    fn push_client_enforces_cap() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("listener addr");
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        let held = connect_clients(&addr, MAX_CLIENTS_PER_PORT + 8);
        let mut accepted = 0;
        for stream in held {
            if NetIo::push_client(&clients, stream) {
                accepted += 1;
            }
        }
        assert_eq!(accepted, MAX_CLIENTS_PER_PORT);
        assert_eq!(clients.lock().expect("lock").len(), MAX_CLIENTS_PER_PORT);
    }

    #[test]
    fn prune_and_send_drops_closed_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener
            .set_nonblocking(true)
            .expect("listener nonblocking");
        let addr = listener.local_addr().expect("listener addr");
        let peer = TcpStream::connect(addr).expect("connect");
        // Accept server side, then close the peer abruptly: the next send
        // must prune the dead client instead of retaining it forever.
        let (server, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("accept failed: {e}"),
            }
        };
        server
            .set_nonblocking(true)
            .expect("server stream nonblocking");
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        assert!(NetIo::push_client(&clients, server));
        drop(peer);
        // Give the kernel a moment to deliver FIN/RST.
        std::thread::sleep(Duration::from_millis(100));
        NetIo::prune_and_send(&clients, &[0x1au8; 64]);
        NetIo::prune_and_send(&clients, &[0x1au8; 64]);
        assert!(
            clients.lock().expect("lock").is_empty(),
            "closed peer must be pruned"
        );
    }

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

    #[test]
    fn beast_frame_indicator_for_2byte_msg() {
        let frame = encode_beast_frame(0, 0, &[0x00, 0x00]);
        assert_eq!(frame[1], 0x31);
    }

    #[test]
    fn beast_frame_indicator_for_7byte_msg() {
        let frame = encode_beast_frame(0, 0, &[0u8; 7]);
        assert_eq!(frame[1], 0x32);
    }

    #[test]
    fn beast_frame_indicator_for_14byte_msg() {
        let frame = encode_beast_frame(0, 0, &[0u8; 14]);
        assert_eq!(frame[1], 0x33);
    }

    #[test]
    fn beast_frame_indicator_defaults_to_long() {
        let frame = encode_beast_frame(0, 0, &[0u8; 20]);
        assert_eq!(frame[1], 0x33);
    }

    #[test]
    fn beast_frame_timestamp_is_6_bytes_be() {
        let frame = encode_beast_frame(0x123456789ABC, 0, &[0u8; 7]);
        // to_be_bytes() is [0x00,0x00,0x12,0x34,0x56,0x78,0x9A,0xBC], sliced from [2..]
        assert_eq!(frame[2], 0x12);
        assert_eq!(frame[3], 0x34);
        assert_eq!(frame[4], 0x56);
        assert_eq!(frame[5], 0x78);
        assert_eq!(frame.len(), 1 + 1 + 6 + 1 + 7);
    }

    #[test]
    fn beast_frame_contains_signal_byte() {
        let frame = encode_beast_frame(0, 0xAB, &[0u8; 7]);
        assert_eq!(frame[8], 0xAB);
    }

    #[test]
    fn hex_digit_0_to_9() {
        assert_eq!(hex_digit(0), b'0');
        assert_eq!(hex_digit(5), b'5');
        assert_eq!(hex_digit(9), b'9');
    }

    #[test]
    fn hex_digit_a_to_f() {
        assert_eq!(hex_digit(10), b'A');
        assert_eq!(hex_digit(15), b'F');
    }

    #[test]
    fn hex_digit_out_of_range() {
        assert_eq!(hex_digit(16), b'?');
        assert_eq!(hex_digit(255), b'?');
    }

    #[test]
    fn hex_digit_all_valid_values() {
        let expected = *b"0123456789ABCDEF";
        for i in 0..=15u8 {
            assert_eq!(hex_digit(i), expected[i as usize], "hex_digit({i})");
        }
    }

    #[test]
    fn beast_frame_empty_message() {
        let frame = encode_beast_frame(0, 0, &[]);
        assert_eq!(frame[0], 0x1a);
        assert_eq!(frame[1], 0x33);
        assert_eq!(frame.len(), 1 + 1 + 6 + 1);
    }

    #[test]
    fn beast_frame_all_0x1a_payload() {
        let msg = [0x1au8; 7];
        let frame = encode_beast_frame(0, 0, &msg);
        // marker, indicator(0x32 for 7-byte), 6 timestamp, 1 signal, 7*2 escaped
        assert_eq!(frame.len(), 23);
        assert_eq!(frame[0], 0x1a);
        // Every msg byte (0x1a) should appear doubled in payload
        let expected_escaping = [
            0x32, 0, 0, 0, 0, 0, 0, 0, 0x1a, 0x1a, 0x1a, 0x1a, 0x1a, 0x1a, 0x1a, 0x1a, 0x1a, 0x1a,
            0x1a, 0x1a, 0x1a, 0x1a,
        ];
        assert_eq!(&frame[1..], &expected_escaping[..]);
    }

    #[test]
    fn beast_frame_all_0x1a_payload_long() {
        let msg = [0x1au8; 14];
        let frame = encode_beast_frame(0, 0, &msg);
        // 1 marker + 1 indicator + 6 ts + 1 sig + 14*2 escaped = 37
        assert_eq!(frame.len(), 37);
        // Count 0x1a bytes: 1 marker + (14 * 2) escaped = 29
        let count = frame.iter().filter(|&&b| b == 0x1a).count();
        assert_eq!(count, 29);
    }

    #[test]
    fn beast_frame_zero_timestamp() {
        let frame = encode_beast_frame(0, 0, &[0u8; 7]);
        assert_eq!(frame[2], 0);
        assert_eq!(frame[3], 0);
        assert_eq!(frame[4], 0);
        assert_eq!(frame[5], 0);
        assert_eq!(frame[6], 0);
        assert_eq!(frame[7], 0);
    }

    #[test]
    fn beast_frame_max_timestamp() {
        // Max 48-bit value
        let frame = encode_beast_frame(0xFFFF_FFFF_FFFF, 0, &[0u8; 7]);
        assert_eq!(frame[2], 0xFF);
        assert_eq!(frame[3], 0xFF);
        assert_eq!(frame[4], 0xFF);
        assert_eq!(frame[5], 0xFF);
        assert_eq!(frame[6], 0xFF);
        assert_eq!(frame[7], 0xFF);
    }

    #[test]
    fn beast_frame_signal_byte_zero() {
        let frame = encode_beast_frame(0, 0, &[0u8; 7]);
        assert_eq!(frame[8], 0);
    }

    #[test]
    fn beast_frame_signal_byte_max() {
        let frame = encode_beast_frame(0, 255, &[0u8; 7]);
        assert_eq!(frame[8], 255);
    }

    #[test]
    fn beast_frame_frame_length_matches_input() {
        for len in [0, 1, 2, 7, 8, 14, 20, 100] {
            let msg = vec![0x42u8; len];
            let frame = encode_beast_frame(0, 0, &msg);
            // base = 1 marker + 1 indicator + 6 ts + 1 signal = 9, no escaping
            assert_eq!(frame.len(), 9 + len, "length mismatch for len={len}");
        }
    }

    #[test]
    fn beast_frame_no_escaping_without_0x1a() {
        let msg = [0x42u8; 7];
        let frame = encode_beast_frame(0, 0, &msg);
        assert_eq!(frame.len(), 16); // 9 + 7
        assert_eq!(&frame[9..], &msg[..]);
    }
}
