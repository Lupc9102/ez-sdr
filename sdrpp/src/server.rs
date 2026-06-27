use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, Shutdown};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

const SERVER_MAX_PACKET_SIZE: usize = 65536;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketType {
    Command = 0,
    CommandAck = 1,
    Error = 2,
    Baseband = 3,
    BasebandCompressed = 4,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Disconnect = 0,
    GetUi = 1,
    UiAction = 2,
    Start = 3,
    Stop = 4,
    SetFrequency = 5,
    SetSampleType = 6,
    SetCompression = 7,
    SetSamplerate = 8,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidPacket = 0,
    InvalidCommand = 1,
    InvalidArgument = 2,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct PacketHeader {
    pub size: u32,
    pub typ: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct CommandHeader {
    pub cmd: u8,
}

pub struct Server {
    listener: Option<TcpListener>,
    client: Arc<Mutex<Option<TcpStream>>>,
    running: Arc<AtomicBool>,
    compression: Arc<AtomicBool>,
    sample_rate: Arc<Mutex<f64>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub fn new() -> Self {
        Self {
            listener: None,
            client: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            compression: Arc::new(AtomicBool::new(false)),
            sample_rate: Arc::new(Mutex::new(1_000_000.0)),
            worker: None,
        }
    }

    pub fn start(&mut self, host: &str, port: u16) -> std::io::Result<()> {
        let addr = format!("{}:{}", host, port);
        let listener = TcpListener::bind(&addr)?;
        listener.set_nonblocking(true)?;
        self.listener = Some(listener);
        self.running.store(true, Ordering::SeqCst);

        let client = Arc::clone(&self.client);
        let running = Arc::clone(&self.running);
        let compression = Arc::clone(&self.compression);
        let sample_rate = Arc::clone(&self.sample_rate);

        if let Some(ref l) = self.listener {
            let listener = l.try_clone()?;
            self.worker = Some(thread::spawn(move || {
                server_loop(listener, client, running, compression, sample_rate);
            }));
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(ref l) = self.listener {
            let _ = l.set_nonblocking(false);
        }
        {
            let mut client = self.client.lock().unwrap();
            if let Some(ref mut c) = *client {
                let _ = c.shutdown(Shutdown::Both);
            }
            *client = None;
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.listener = None;
    }

    pub fn set_compression(&self, enabled: bool) {
        self.compression.store(enabled, Ordering::Relaxed);
    }

    pub fn is_compression_enabled(&self) -> bool {
        self.compression.load(Ordering::Relaxed)
    }

    pub fn set_sample_rate(&self, sr: f64) {
        let mut guard = self.sample_rate.lock().unwrap();
        *guard = sr;
        drop(guard);
        let mut client = self.client.lock().unwrap();
        if let Some(ref mut c) = *client {
            let mut sbuf = [0u8; SERVER_MAX_PACKET_SIZE];
            let s_cmd_data = &mut sbuf[std::mem::size_of::<PacketHeader>() + std::mem::size_of::<CommandHeader>()..];
            s_cmd_data[..8].copy_from_slice(&sr.to_le_bytes());
            let s_pkt_hdr = unsafe { &mut *(sbuf.as_mut_ptr() as *mut PacketHeader) };
            let s_cmd_hdr = unsafe { &mut *(sbuf.as_mut_ptr().add(std::mem::size_of::<PacketHeader>()) as *mut CommandHeader) };
            s_cmd_hdr.cmd = Command::SetSamplerate as u8;
            s_pkt_hdr.typ = PacketType::CommandAck as u8;
            s_pkt_hdr.size = (std::mem::size_of::<PacketHeader>() + std::mem::size_of::<CommandHeader>() + 8) as u32;
            let _ = c.write_all(&sbuf[..s_pkt_hdr.size as usize]);
        }
    }

    pub fn send_baseband(&self, data: &[u8]) {
        let mut client = self.client.lock().unwrap();
        if let Some(ref mut c) = *client {
            let compression = self.compression.load(Ordering::Relaxed);
            let mut bbuf = [0u8; SERVER_MAX_PACKET_SIZE];
            let bb_pkt_hdr = unsafe { &mut *(bbuf.as_mut_ptr() as *mut PacketHeader) };
            if compression {
                bb_pkt_hdr.typ = PacketType::BasebandCompressed as u8;
                bb_pkt_hdr.size = std::mem::size_of::<PacketHeader>() as u32;
            } else {
                bb_pkt_hdr.typ = PacketType::Baseband as u8;
                bb_pkt_hdr.size = (std::mem::size_of::<PacketHeader>() + data.len()) as u32;
                let payload = &mut bbuf[std::mem::size_of::<PacketHeader>()..std::mem::size_of::<PacketHeader>() + data.len()];
                payload.copy_from_slice(data);
            }
            let _ = c.write_all(&bbuf[..bb_pkt_hdr.size as usize]);
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

fn server_loop(
    listener: TcpListener,
    client: Arc<Mutex<Option<TcpStream>>>,
    running: Arc<AtomicBool>,
    compression: Arc<AtomicBool>,
    sample_rate: Arc<Mutex<f64>>,
) {
    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut guard = client.lock().unwrap();
                if guard.is_some() {
                    let mut buf = [0u8; std::mem::size_of::<PacketHeader>() + std::mem::size_of::<CommandHeader>()];
                    let tmp_phdr = unsafe { &mut *(buf.as_mut_ptr() as *mut PacketHeader) };
                    let tmp_chdr = unsafe { &mut *(buf.as_mut_ptr().add(std::mem::size_of::<PacketHeader>()) as *mut CommandHeader) };
                    tmp_phdr.size = buf.len() as u32;
                    tmp_phdr.typ = PacketType::Command as u8;
                    tmp_chdr.cmd = Command::Disconnect as u8;
                    let _ = stream.write_all(&buf);
                    let _ = stream.shutdown(Shutdown::Both);
                    drop(guard);
                    continue;
                }
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let _ = stream.set_nodelay(true);
                *guard = Some(stream);
                drop(guard);

                let client2 = Arc::clone(&client);
                let running2 = Arc::clone(&running);
                let compression2 = Arc::clone(&compression);
                let sr2 = Arc::clone(&sample_rate);
                thread::spawn(move || {
                    handle_client(client2, running2, compression2, sr2);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => {}
        }
    }
}

fn handle_client(
    client: Arc<Mutex<Option<TcpStream>>>,
    running: Arc<AtomicBool>,
    compression: Arc<AtomicBool>,
    sample_rate: Arc<Mutex<f64>>,
) {
    {
        let mut guard = client.lock().unwrap();
        if let Some(ref mut c) = *guard {
            let sr = *sample_rate.lock().unwrap();
            let mut sbuf = [0u8; SERVER_MAX_PACKET_SIZE];
            let s_cmd_data = &mut sbuf[std::mem::size_of::<PacketHeader>() + std::mem::size_of::<CommandHeader>()..];
            s_cmd_data[..8].copy_from_slice(&sr.to_le_bytes());
            let s_pkt_hdr = unsafe { &mut *(sbuf.as_mut_ptr() as *mut PacketHeader) };
            let s_cmd_hdr = unsafe { &mut *(sbuf.as_mut_ptr().add(std::mem::size_of::<PacketHeader>()) as *mut CommandHeader) };
            s_cmd_hdr.cmd = Command::SetSamplerate as u8;
            s_pkt_hdr.typ = PacketType::CommandAck as u8;
            s_pkt_hdr.size = (std::mem::size_of::<PacketHeader>() + std::mem::size_of::<CommandHeader>() + 8) as u32;
            let _ = c.write_all(&sbuf[..s_pkt_hdr.size as usize]);
        }
    }

    let mut rbuf = [0u8; SERVER_MAX_PACKET_SIZE];
    while running.load(Ordering::SeqCst) {
        let mut stream_opt = {
            let mut guard = client.lock().unwrap();
            guard.take()
        };
        let stream = match stream_opt {
            Some(ref mut s) => s,
            None => break,
        };

        let hdr_size = std::mem::size_of::<PacketHeader>();
        let mut read = 0usize;
        while read < hdr_size {
            match stream.read(&mut rbuf[read..hdr_size]) {
                Ok(0) => {
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
                Ok(n) => read += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(_) => {
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
            }
        }

        let pkt_hdr = unsafe { &*(rbuf.as_ptr() as *const PacketHeader) };
        let total = pkt_hdr.size as usize;
        if total > SERVER_MAX_PACKET_SIZE {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }

        while read < total {
            match stream.read(&mut rbuf[read..total]) {
                Ok(0) => {
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
                Ok(n) => read += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(_) => {
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
            }
        }

        if pkt_hdr.typ == PacketType::Command as u8 && total >= hdr_size + std::mem::size_of::<CommandHeader>() {
            let cmd_hdr = unsafe { &*(rbuf.as_ptr().add(hdr_size) as *const CommandHeader) };
            let data = &rbuf[hdr_size + std::mem::size_of::<CommandHeader>()..total];
            let cmd = match cmd_hdr.cmd {
                0 => Command::Disconnect,
                1 => Command::GetUi,
                2 => Command::UiAction,
                3 => Command::Start,
                4 => Command::Stop,
                5 => Command::SetFrequency,
                6 => Command::SetSampleType,
                7 => Command::SetCompression,
                8 => Command::SetSamplerate,
                _ => Command::Disconnect,
            };
            handle_command(cmd, data, stream, &compression, &sample_rate);
        } else {
            send_error(stream, ErrorCode::InvalidPacket);
        }

        {
            let mut guard = client.lock().unwrap();
            *guard = stream_opt.take();
        }
    }
}

fn handle_command(
    cmd: Command,
    data: &[u8],
    stream: &mut TcpStream,
    compression: &AtomicBool,
    sample_rate: &Mutex<f64>,
) {
    match cmd {
        Command::GetUi => {
            send_command_ack(stream, Command::GetUi, 0);
        }
        Command::UiAction => {
            if data.len() >= 3 {
                send_command_ack(stream, Command::UiAction, 0);
            } else {
                send_error(stream, ErrorCode::InvalidArgument);
            }
        }
        Command::Start => {
            let mut _sr = sample_rate.lock().unwrap();
        }
        Command::Stop => {
            let mut _sr = sample_rate.lock().unwrap();
        }
        Command::SetFrequency => {
            if data.len() == 8 {
                let freq = f64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
                let mut sr = sample_rate.lock().unwrap();
                *sr = freq;
                drop(sr);
                send_command_ack(stream, Command::SetFrequency, 0);
            } else {
                send_error(stream, ErrorCode::InvalidArgument);
            }
        }
        Command::SetSampleType => {
            if data.len() == 1 {
            }
        }
        Command::SetCompression => {
            if data.len() == 1 {
                compression.store(data[0] != 0, Ordering::Relaxed);
            }
        }
        _ => {
            send_error(stream, ErrorCode::InvalidCommand);
        }
    }
}

fn send_packet(stream: &mut TcpStream, typ: PacketType, data: &[u8]) {
    let mut buf = [0u8; SERVER_MAX_PACKET_SIZE];
    let hdr = unsafe { &mut *(buf.as_mut_ptr() as *mut PacketHeader) };
    hdr.typ = typ as u8;
    hdr.size = (std::mem::size_of::<PacketHeader>() + data.len()) as u32;
    let payload = &mut buf[std::mem::size_of::<PacketHeader>()..std::mem::size_of::<PacketHeader>() + data.len()];
    payload.copy_from_slice(data);
    let _ = stream.write_all(&buf[..hdr.size as usize]);
}

fn send_command_ack(stream: &mut TcpStream, cmd: Command, len: usize) {
    let mut buf = [0u8; SERVER_MAX_PACKET_SIZE];
    let pkt_hdr = unsafe { &mut *(buf.as_mut_ptr() as *mut PacketHeader) };
    let cmd_hdr = unsafe { &mut *(buf.as_mut_ptr().add(std::mem::size_of::<PacketHeader>()) as *mut CommandHeader) };
    cmd_hdr.cmd = cmd as u8;
    pkt_hdr.typ = PacketType::CommandAck as u8;
    pkt_hdr.size = (std::mem::size_of::<PacketHeader>() + std::mem::size_of::<CommandHeader>() + len) as u32;
    let _ = stream.write_all(&buf[..pkt_hdr.size as usize]);
}

fn send_error(stream: &mut TcpStream, err: ErrorCode) {
    let data = [err as u8];
    send_packet(stream, PacketType::Error, &data);
}
