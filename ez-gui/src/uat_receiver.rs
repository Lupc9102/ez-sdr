//! Bounded, cancellable ingestion of FlightAware dump978-fa's newline JSON feed.
//! The decoder owns the SDR; this module only reads decoded aircraft reports.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const DEFAULT_ADDRESS: &str = "127.0.0.1:30979";
pub const SETUP_COMMAND: &str = "dump978-fa --sdr driver=rtlsdr --json-port 127.0.0.1:30979";
const MAX_LINE_BYTES: usize = 16 * 1024;
const QUEUE_CAPACITY: usize = 512;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const READ_TIMEOUT: Duration = Duration::from_millis(150);
const RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq)]
pub struct UatReport {
    /// A tracking key: ICAO occupies 24 bits; other address classes are namespaced.
    pub key: u32,
    pub callsign: Option<String>,
    pub position: Option<(f64, f64)>,
    pub altitude_ft: Option<i32>,
    pub ground_speed_kt: Option<f64>,
    pub heading_deg: Option<f64>,
}

pub fn address_label(key: u32) -> String {
    if key <= 0x00ff_ffff {
        format!("{key:06X}")
    } else {
        format!("UAT {:06X}", key & 0x00ff_ffff)
    }
}

/// Parse the direct `AdsbMessage::ToJson()` output, not SkyAware's file format.
/// Unknown qualifiers, malformed addresses and invalid coordinates are rejected.
pub fn parse_report(line: &[u8]) -> Result<UatReport, String> {
    if line.len() > MAX_LINE_BYTES {
        return Err("UAT JSON line exceeds 16 KiB".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(line).map_err(|e| format!("Invalid UAT JSON: {e}"))?;
    let object = value.as_object().ok_or("UAT JSON must be an object")?;
    let address = object
        .get("address")
        .and_then(serde_json::Value::as_str)
        .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("UAT address must contain exactly six hex digits")?;
    let address = u32::from_str_radix(address, 16).map_err(|e| e.to_string())?;
    let qualifier = object
        .get("address_qualifier")
        .and_then(serde_json::Value::as_str)
        .ok_or("Missing UAT address qualifier")?;
    let namespace = match qualifier {
        "adsb_icao" | "tisb_icao" => 0,
        "adsb_other" => 1,
        "tisb_trackfile" => 2,
        "vehicle" => 3,
        "fixed_beacon" => 4,
        "adsr_other" => 5,
        _ => return Err(format!("Unsupported UAT address qualifier: {qualifier}")),
    };
    let number = |name: &str, min: f64, max: f64| -> Result<Option<f64>, String> {
        match object.get(name) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(value) => value
                .as_f64()
                .filter(|v| v.is_finite() && *v >= min && *v <= max)
                .map(Some)
                .ok_or_else(|| format!("Invalid UAT {name}")),
        }
    };
    let position = match object.get("position") {
        None | Some(serde_json::Value::Null) => None,
        Some(position) => {
            let lat = position.get("lat").and_then(serde_json::Value::as_f64);
            let lon = position.get("lon").and_then(serde_json::Value::as_f64);
            match (lat, lon) {
                (Some(lat), Some(lon))
                    if lat.is_finite()
                        && lon.is_finite()
                        && (-90.0..=90.0).contains(&lat)
                        && (-180.0..=180.0).contains(&lon) =>
                {
                    Some((lat, lon))
                }
                _ => return Err("Invalid UAT position".into()),
            }
        }
    };
    let callsign = match object.get("callsign") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            let raw = value.as_str().ok_or("Invalid UAT callsign")?;
            if raw.len() > 16 || raw.chars().any(char::is_control) {
                return Err("Invalid UAT callsign".into());
            }
            let trimmed = raw.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
    };
    let pressure = number("pressure_altitude", -2_000.0, 100_000.0)?;
    let geometric = number("geometric_altitude", -2_000.0, 100_000.0)?;
    let track = number("true_track", 0.0, 360.0)?;
    let heading = number("true_heading", 0.0, 360.0)?;
    Ok(UatReport {
        key: (namespace << 24) | address,
        callsign,
        position,
        altitude_ft: pressure.or(geometric).map(|v| v.round() as i32),
        ground_speed_kt: number("ground_speed", 0.0, 3_000.0)?,
        heading_deg: track.or(heading).map(|v| v.rem_euclid(360.0)),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UatStatus {
    Stopped,
    Connecting,
    Connected,
    Reconnecting(String),
    Error(String),
}

#[derive(Default)]
struct Counters {
    received: AtomicU64,
    rejected: AtomicU64,
    dropped: AtomicU64,
}

#[derive(Clone, Debug)]
pub struct TimedReport {
    pub report: UatReport,
    pub received: Instant,
}

pub struct UatReceiver {
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    status: Arc<Mutex<UatStatus>>,
    counters: Arc<Counters>,
    rx: crossbeam_channel::Receiver<TimedReport>,
}

impl Default for UatReceiver {
    fn default() -> Self {
        let (_, rx) = crossbeam_channel::bounded(QUEUE_CAPACITY);
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
            status: Arc::new(Mutex::new(UatStatus::Stopped)),
            counters: Arc::new(Counters::default()),
            rx,
        }
    }
}

impl UatReceiver {
    pub fn start(&mut self, endpoint: &str) -> Result<(), String> {
        self.stop();
        self.counters = Arc::new(Counters::default());
        // Numeric endpoints keep DNS resolution, which has no std timeout, out
        // of both the UI thread and the cancellation lifecycle.
        let endpoint = endpoint.trim().parse::<SocketAddr>().map_err(|_| {
            let message = "Use an IP address and port, e.g. 127.0.0.1:30979".to_owned();
            self.set_status(UatStatus::Error(message.clone()));
            message
        })?;
        if endpoint.port() == 0 || endpoint.ip().is_unspecified() || endpoint.ip().is_multicast() {
            let message = "Use a destination IP address and a nonzero TCP port".to_owned();
            self.set_status(UatStatus::Error(message.clone()));
            return Err(message);
        }
        let (tx, rx) = crossbeam_channel::bounded(QUEUE_CAPACITY);
        self.rx = rx;
        self.cancel = Arc::new(AtomicBool::new(false));
        // A new status object prevents a canceled worker from overwriting its successor.
        self.status = Arc::new(Mutex::new(UatStatus::Connecting));
        let cancel = self.cancel.clone();
        let status = self.status.clone();
        let counters = self.counters.clone();
        let handle = thread::Builder::new()
            .name("uat-json-receiver".into())
            .spawn(move || receiver_loop(endpoint, cancel, status, counters, tx))
            .map_err(|e| {
                let message = format!("Could not start UAT receiver: {e}");
                self.set_status(UatStatus::Error(message.clone()));
                message
            })?;
        self.worker = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            // Never join a connecting socket from the UI thread. The canceled
            // worker exits within CONNECT_TIMEOUT/READ_TIMEOUT and owns no UI state.
        }
        let (_, rx) = crossbeam_channel::bounded(QUEUE_CAPACITY);
        self.rx = rx;
        // Canceled workers retain the old queue/status; a final in-flight read
        // cannot leak stale reports or overwrite the disconnected state.
        self.status = Arc::new(Mutex::new(UatStatus::Stopped));
    }

    pub fn is_active(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| !w.is_finished())
    }

    pub fn status(&self) -> UatStatus {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            return UatStatus::Error("UAT receive worker stopped; reconnect to restart".into());
        }
        self.status
            .lock()
            .map(|v| v.clone())
            .unwrap_or_else(|_| UatStatus::Error("UAT receiver status is unavailable".into()))
    }

    pub fn counts(&self) -> (u64, u64, u64) {
        (
            self.counters.received.load(Ordering::Relaxed),
            self.counters.rejected.load(Ordering::Relaxed),
            self.counters.dropped.load(Ordering::Relaxed),
        )
    }

    pub fn try_recv(&self) -> Option<TimedReport> {
        self.rx.try_recv().ok()
    }

    fn set_status(&self, value: UatStatus) {
        set_status(&self.status, value);
    }
}

impl Drop for UatReceiver {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set_status(status: &Mutex<UatStatus>, value: UatStatus) {
    if let Ok(mut status) = status.lock() {
        *status = value;
    }
}

/// Keeps only one bounded line even if a misconfigured endpoint sends binary
/// data or never terminates a line. Recover after the next newline.
#[derive(Default)]
struct LineBuffer {
    bytes: Vec<u8>,
    overlong: bool,
}

impl LineBuffer {
    fn feed(&mut self, input: &[u8], mut on_line: impl FnMut(Result<&[u8], ()>)) {
        for &byte in input {
            if byte == b'\n' {
                if self.overlong {
                    on_line(Err(()));
                } else if !self.bytes.is_empty() {
                    on_line(Ok(&self.bytes));
                }
                self.bytes.clear();
                self.overlong = false;
            } else if !self.overlong {
                if self.bytes.len() == MAX_LINE_BYTES {
                    self.bytes.clear();
                    self.overlong = true;
                } else {
                    self.bytes.push(byte);
                }
            }
        }
    }
}

fn read_reports(
    reader: &mut impl Read,
    cancel: &AtomicBool,
    counters: &Counters,
    tx: &crossbeam_channel::Sender<TimedReport>,
) -> io::Result<()> {
    let mut buffer = [0_u8; 4096];
    let mut lines = LineBuffer::default();
    while !cancel.load(Ordering::Acquire) {
        match reader.read(&mut buffer) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "decoder closed the stream",
                ))
            }
            Ok(count) => lines.feed(&buffer[..count], |line| {
                let report = line
                    .map_err(|()| "UAT JSON line exceeds 16 KiB".to_owned())
                    .and_then(parse_report);
                match report {
                    Ok(report) => {
                        counters.received.fetch_add(1, Ordering::Relaxed);
                        if tx
                            .try_send(TimedReport {
                                report,
                                received: Instant::now(),
                            })
                            .is_err()
                        {
                            counters.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(_) => {
                        counters.rejected.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn receiver_loop(
    endpoint: SocketAddr,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<UatStatus>>,
    counters: Arc<Counters>,
    tx: crossbeam_channel::Sender<TimedReport>,
) {
    while !cancel.load(Ordering::Acquire) {
        let result =
            TcpStream::connect_timeout(&endpoint, CONNECT_TIMEOUT).and_then(|mut stream| {
                stream.set_read_timeout(Some(READ_TIMEOUT))?;
                if cancel.load(Ordering::Acquire) {
                    return Ok(());
                }
                set_status(&status, UatStatus::Connected);
                read_reports(&mut stream, &cancel, &counters, &tx)
            });
        if cancel.load(Ordering::Acquire) {
            break;
        }
        if let Err(error) = result {
            set_status(
                &status,
                UatStatus::Reconnecting(format!("{endpoint}: {error}")),
            );
        }
        thread::park_timeout(RETRY_DELAY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &[u8] = br#"{"address":"a1b2c3","address_qualifier":"adsb_icao","position":{"lat":40.25,"lon":-75.5},"callsign":" N123AB ","pressure_altitude":12000,"ground_speed":142.5,"true_track":90}"#;

    #[test]
    fn direct_dump978_report_fields_and_units() {
        let report = parse_report(REPORT).unwrap();
        assert_eq!(report.key, 0xa1b2c3);
        assert_eq!(report.callsign.as_deref(), Some("N123AB"));
        assert_eq!(report.position, Some((40.25, -75.5)));
        assert_eq!(report.altitude_ft, Some(12000));
        assert_eq!(report.ground_speed_kt, Some(142.5));
        assert_eq!(report.heading_deg, Some(90.0));
    }

    #[test]
    fn partial_updates_and_geometric_heading_fallback() {
        let report = parse_report(br#"{"address":"000001","address_qualifier":"tisb_icao","geometric_altitude":-100,"true_heading":360}"#).unwrap();
        assert_eq!(report.key, 1);
        assert_eq!(report.position, None);
        assert_eq!(report.callsign, None);
        assert_eq!(report.altitude_ft, Some(-100));
        assert_eq!(report.heading_deg, Some(0.0));
    }

    #[test]
    fn non_icao_tracks_do_not_collide_or_look_like_icao() {
        let report =
            parse_report(br#"{"address":"a1b2c3","address_qualifier":"adsb_other"}"#).unwrap();
        assert_ne!(report.key, 0xa1b2c3);
        assert!(report.key > 0xffffff);
        assert_eq!(address_label(report.key), "UAT A1B2C3");
    }

    #[test]
    fn invalid_fields_are_rejected_and_zero_coordinates_are_valid() {
        let base: serde_json::Value = serde_json::from_slice(REPORT).unwrap();
        for (field, value) in [
            ("address", serde_json::json!("ABC")),
            ("address", serde_json::json!("ABCXYZ")),
            ("address_qualifier", serde_json::json!("reserved")),
            ("position", serde_json::json!({"lat": 91, "lon": 0})),
            ("position", serde_json::json!({"lat": 40})),
            ("ground_speed", serde_json::json!(-1)),
            ("true_track", serde_json::json!(400)),
            ("callsign", serde_json::json!("BAD\nTEXT")),
        ] {
            let mut value_to_test = base.clone();
            value_to_test[field] = value;
            assert!(
                parse_report(&serde_json::to_vec(&value_to_test).unwrap()).is_err(),
                "accepted invalid {field}"
            );
        }
        let mut zero = base;
        zero["position"] = serde_json::json!({"lat":0, "lon":0});
        assert_eq!(
            parse_report(&serde_json::to_vec(&zero).unwrap())
                .unwrap()
                .position,
            Some((0.0, 0.0))
        );
    }

    #[test]
    fn fragmented_lines_crlf_and_oversized_lines_recover() {
        let mut buffer = LineBuffer::default();
        let mut results = Vec::new();
        buffer.feed(b"{\"a\":", |r| results.push(r.map(Vec::from)));
        assert!(results.is_empty());
        buffer.feed(b"1}\r\n", |r| results.push(r.map(Vec::from)));
        assert_eq!(results[0].as_ref().unwrap(), b"{\"a\":1}\r");
        buffer.feed(&vec![b'x'; MAX_LINE_BYTES * 3], |_| unreachable!());
        assert!(buffer.bytes.capacity() <= MAX_LINE_BYTES);
        assert!(buffer.bytes.is_empty());
        buffer.feed(b"\n{}\n", |r| results.push(r.map(Vec::from)));
        assert_eq!(results[1], Err(()));
        assert_eq!(results[2], Ok(b"{}".to_vec()));
    }

    #[test]
    fn reader_bounds_queue_and_counts_rejections() {
        let mut data = Vec::new();
        for _ in 0..3 {
            data.extend_from_slice(REPORT);
            data.push(b'\n');
        }
        data.extend_from_slice(b"not json\n");
        let counters = Counters::default();
        let (tx, rx) = crossbeam_channel::bounded(2);
        let error = read_reports(
            &mut io::Cursor::new(data),
            &AtomicBool::new(false),
            &counters,
            &tx,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(rx.len(), 2);
        assert_eq!(counters.received.load(Ordering::Relaxed), 3);
        assert_eq!(counters.rejected.load(Ordering::Relaxed), 1);
        assert_eq!(counters.dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cancellation_stops_before_read_and_invalid_endpoint_never_starts() {
        struct MustNotRead;
        impl Read for MustNotRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("canceled worker read")
            }
        }
        let (tx, _) = crossbeam_channel::bounded(1);
        read_reports(
            &mut MustNotRead,
            &AtomicBool::new(true),
            &Counters::default(),
            &tx,
        )
        .unwrap();
        let mut receiver = UatReceiver::default();
        for endpoint in [
            "localhost:30979",
            "127.0.0.1:0",
            "0.0.0.0:30979",
            "not-an-address",
        ] {
            assert!(receiver.start(endpoint).is_err());
            assert!(!receiver.is_active());
            assert!(matches!(receiver.status(), UatStatus::Error(_)));
        }
        receiver.stop();
        assert_eq!(receiver.status(), UatStatus::Stopped);
    }

    #[test]
    fn tcp_reconnect_and_cancellation_lifecycle() {
        use std::io::Write;
        use std::net::TcpListener;
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("loopback TCP required for lifecycle test");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let start = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(
                                start.elapsed() < Duration::from_secs(8),
                                "receiver never connected"
                            );
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("accept failed: {error}"),
                    }
                };
                stream.write_all(&REPORT[..30]).unwrap();
                stream.write_all(&REPORT[30..]).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        });
        let mut receiver = UatReceiver::default();
        receiver.start(&address.to_string()).unwrap();
        let start = Instant::now();
        let mut reports = 0;
        while reports < 2 && start.elapsed() < Duration::from_secs(8) {
            if let Some(report) = receiver.try_recv() {
                assert_eq!(report.report.key, 0xa1b2c3);
                reports += 1;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(reports, 2, "decoder reconnect did not deliver both reports");
        let worker = receiver.worker.take().unwrap();
        let start = Instant::now();
        receiver.cancel.store(true, Ordering::Release);
        worker.thread().unpark();
        worker.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        receiver.stop();
        assert_eq!(receiver.status(), UatStatus::Stopped);
        server.join().unwrap();
    }
}
