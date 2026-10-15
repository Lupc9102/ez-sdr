use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const SAMPLE_RATE: usize = 2_400_000;
const FRAME_SAMPLES: usize = 48_000;
const CALLSIGN_FRAME: [u8; 14] = [
    0x8D, 0x48, 0x40, 0xD6, 0x20, 0x2C, 0xC3, 0x7C, 0xDB, 0x31, 0x57, 0x9F, 0xE8, 0x02,
];
const AIRBORNE_FRAME: [u8; 14] = [
    0x8D, 0x40, 0x62, 0x1D, 0x58, 0xC3, 0x82, 0xD6, 0x90, 0xC8, 0xAC, 0x28, 0x63, 0xA7,
];
const AIRBORNE_ODD_FRAME: [u8; 14] = [
    0x8D, 0x40, 0x62, 0x1D, 0x58, 0xC3, 0x86, 0x43, 0x5C, 0xC4, 0x12, 0x69, 0x2A, 0xD6,
];

#[test]
fn cli_streams_beast_raw_and_populated_sbs_from_real_synthetic_iq() {
    let [beast_port, sbs_port, raw_port] = free_ports();
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_dump1090"))
            .args([
                "--ifile",
                "-",
                "--net",
                "--net-bind-address",
                "127.0.0.1",
                "--net-beast-port",
                &beast_port.to_string(),
                "--net-sbs-port",
                &sbs_port.to_string(),
                "--net-raw-port",
                &raw_port.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start dump1090 executable"),
    );

    // Establish all three independent clients before any IQ samples are sent.
    let mut beast = connect_when_ready(beast_port);
    let mut sbs = connect_when_ready(sbs_port);
    let mut raw = connect_when_ready(raw_port);
    for stream in [&beast, &sbs, &raw] {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set feed read timeout");
    }

    let mut iq = Vec::new();
    for frame in [
        CALLSIGN_FRAME,
        AIRBORNE_FRAME,
        AIRBORNE_ODD_FRAME,
        velocity_frame(),
    ] {
        iq.extend(synthetic_frame(&frame));
    }
    child
        .0
        .stdin
        .take()
        .expect("child stdin")
        .write_all(&iq)
        .expect("write synthetic IQ to child");
    let status = wait_for_exit(&mut child.0);
    assert!(status.success(), "dump1090 exited with {status}");

    let beast_bytes = read_all(&mut beast);
    let raw_text = String::from_utf8(read_all(&mut raw)).expect("raw AVR is ASCII");
    let sbs_text = String::from_utf8(read_all(&mut sbs)).expect("SBS is ASCII");
    for line in sbs_text.lines() {
        let fields = line.split(',').collect::<Vec<_>>();
        assert_eq!(fields.len(), 22, "BaseStation field count: {line}");
        assert!(valid_sbs_date(fields[6]), "generated date: {line}");
        assert!(valid_sbs_time(fields[7]), "generated time: {line}");
        assert_eq!(fields[6], fields[8], "generated/logged date: {line}");
        assert_eq!(fields[7], fields[9], "generated/logged time: {line}");
    }

    assert!(beast_bytes.len() > 10, "Beast feed should contain frames");
    assert!(
        beast_bytes.contains(&0x1A),
        "Beast marker should be present"
    );
    assert!(
        beast_bytes
            .windows(CALLSIGN_FRAME.len())
            .any(|window| window == CALLSIGN_FRAME),
        "Beast feed should contain the callsign frame"
    );
    assert!(
        raw_text.contains("*8D4840D6202CC37CDB31579FE802;"),
        "raw feed: {raw_text:?}"
    );
    assert!(
        raw_text.contains("*8D40621D58C382D690C8AC2863A7;"),
        "raw feed: {raw_text:?}"
    );

    let callsign = sbs_text
        .lines()
        .map(|line| line.split(',').collect::<Vec<_>>())
        .find(|fields| fields.len() == 22 && fields[1] == "1")
        .unwrap_or_else(|| panic!("SBS identification record missing: {sbs_text:?}"));
    assert_eq!(callsign[0], "MSG");
    assert_eq!(callsign[4], "4840D6");
    assert!(
        valid_sbs_date(callsign[6]),
        "generated date: {}",
        callsign[6]
    );
    assert!(
        valid_sbs_time(callsign[7]),
        "generated time: {}",
        callsign[7]
    );
    assert_eq!(callsign[6], callsign[8], "generated/logged date");
    assert_eq!(callsign[7], callsign[9], "generated/logged time");
    assert_eq!(callsign[10], "KLM 63EW");
    assert!(callsign[17..=20].iter().all(|field| field.is_empty()));
    assert!(callsign[21].is_empty());

    let airborne = sbs_text
        .lines()
        .map(|line| line.split(',').collect::<Vec<_>>())
        .find(|fields| fields.len() == 22 && fields[1] == "3" && !fields[14].is_empty())
        .expect("SBS airborne record with resolved CPR position");
    assert_eq!(airborne[4], "40621D");
    assert_eq!(airborne[11], "38000");
    assert_eq!(airborne[21], "0");
    let latitude = airborne[14].parse::<f64>().expect("SBS latitude");
    let longitude = airborne[15].parse::<f64>().expect("SBS longitude");
    assert!(
        (latitude - 52.26578).abs() < 0.00001,
        "latitude: {latitude}"
    );
    assert!(
        (longitude - 3.93891).abs() < 0.00001,
        "longitude: {longitude}"
    );

    let velocity = sbs_text
        .lines()
        .map(|line| line.split(',').collect::<Vec<_>>())
        .find(|fields| fields.len() == 22 && fields[1] == "4")
        .expect("SBS airborne-velocity record");
    assert_eq!(velocity[4], "40621D");
    let ground_speed = velocity[12].parse::<f64>().expect("ground speed");
    let track = velocity[13].parse::<f64>().expect("track");
    assert!(
        (447.0..448.0).contains(&ground_speed),
        "ground speed: {ground_speed}"
    );
    assert!((63.0..64.0).contains(&track), "track: {track}");
    assert!(velocity[17].is_empty(), "squawk field should be empty");
    assert!(velocity[18..=20].iter().all(|field| field.is_empty()));
    assert_eq!(velocity[21], "0");
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_ports() -> [u16; 3] {
    let listeners = (0..3)
        .map(|_| TcpListener::bind("127.0.0.1:0").expect("reserve loopback port"))
        .collect::<Vec<_>>();
    let ports = listeners
        .iter()
        .map(|listener| listener.local_addr().expect("read reserved port").port())
        .collect::<Vec<_>>();
    drop(listeners);
    [ports[0], ports[1], ports[2]]
}

fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().expect("poll decoder child") {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "decoder process exceeded deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn connect_when_ready(port: u16) -> TcpStream {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(100)) {
            return stream;
        }
        assert!(Instant::now() < deadline, "listener {addr} did not start");
        thread::sleep(Duration::from_millis(10));
    }
}

fn synthetic_frame(frame: &[u8; 14]) -> Vec<u8> {
    let bits = frame
        .iter()
        .flat_map(|byte| (0..8).rev().map(move |shift| (byte >> shift) & 1))
        .collect::<Vec<_>>();
    let mut pulses = vec![(0.0, 0.5), (1.0, 1.5), (3.5, 4.0), (4.5, 5.0)];
    pulses.extend(bits.iter().enumerate().map(|(index, bit)| {
        let start = 8.0 + index as f64 + if *bit == 1 { 0.0 } else { 0.5 };
        (start, start + 0.5)
    }));

    let mut iq = Vec::with_capacity(FRAME_SAMPLES * 2);
    for sample in 0..FRAME_SAMPLES {
        let low_us = (sample as f64 - 200.0 - 0.375) / (SAMPLE_RATE as f64 / 1_000_000.0);
        let high_us = low_us + 1_000_000.0 / SAMPLE_RATE as f64;
        let pulse_fraction = if (0.0..121.0).contains(&low_us) {
            pulses
                .iter()
                .map(|(start, end)| (high_us.min(*end) - low_us.max(*start)).max(0.0))
                .sum::<f64>()
                / (high_us - low_us)
        } else {
            0.0
        };
        iq.push((127.4 + 127.4 * 0.65 * pulse_fraction).round() as u8);
        iq.push(127);
    }
    iq
}

fn velocity_frame() -> [u8; 14] {
    let mut frame = [0_u8; 14];
    frame[..9].copy_from_slice(&[
        0x8D,
        0x40,
        0x62,
        0x1D,
        (19 << 3) | 1,
        0x01,
        0x91,
        0x19,
        0x20,
    ]);
    let parity = independent_crc24(&frame[..11]);
    frame[11] = (parity >> 16) as u8;
    frame[12] = (parity >> 8) as u8;
    frame[13] = parity as u8;
    frame
}

fn independent_crc24(bytes: &[u8]) -> u32 {
    const POLYNOMIAL: u32 = 0x00FF_F409;
    let mut remainder = 0_u32;
    for &byte in bytes {
        remainder ^= u32::from(byte) << 16;
        for _ in 0..8 {
            remainder <<= 1;
            if remainder & 0x0100_0000 != 0 {
                remainder ^= POLYNOMIAL;
            }
        }
    }
    remainder & 0x00FF_FFFF
}

fn read_all(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("read feed to EOF");
    bytes
}

fn valid_sbs_date(value: &str) -> bool {
    let parts = value.split('/').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts.iter().all(|part| part.parse::<u32>().is_ok())
}

fn valid_sbs_time(value: &str) -> bool {
    let Some((whole, millis)) = value.split_once('.') else {
        return false;
    };
    whole.len() == 8
        && millis.len() == 3
        && whole
            .split(':')
            .all(|part| part.len() == 2 && part.parse::<u32>().is_ok())
        && millis.parse::<u32>().is_ok()
}
