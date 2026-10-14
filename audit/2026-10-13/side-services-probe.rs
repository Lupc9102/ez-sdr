use ez_gui::{
    discord::DiscordNotifier,
    rigctl::{RigctlRequest, RigctlServer},
};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    thread,
    time::{Duration, Instant},
};

fn task_ticks() -> Vec<(String, u64)> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
        if let Ok(text) = std::fs::read_to_string(entry.path().join("stat")) {
            let fields: Vec<_> = text
                .rsplit_once(") ")
                .unwrap()
                .1
                .split_whitespace()
                .collect();
            entries.push((
                entry.file_name().to_string_lossy().into(),
                fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap(),
            ));
        }
    }
    entries.sort();
    entries
}

fn main() {
    let mut server = RigctlServer::new();
    server.set_enabled(true, 0);
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.bound_port() == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let mut socket = TcpStream::connect(("127.0.0.1", server.bound_port())).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reader = BufReader::new(socket.try_clone().unwrap());
    socket.write_all(b"v\n").unwrap();
    loop {
        if let Some(request) = server.poll_requests().into_iter().next() {
            match request {
                RigctlRequest::GetVolume { reply } => {
                    println!("Hamlib v (get VFO) parsed as GetVolume");
                    reply.send("70.0\n".into()).unwrap();
                }
                _ => panic!("unexpected request"),
            }
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let mut response = String::new();
    reader.read_line(&mut response).unwrap();
    println!("v response: {:?}", response.trim());
    for command in ["V VFOB\n", "l AF\n", "L AF 0.5\n"] {
        socket.write_all(command.as_bytes()).unwrap();
        response.clear();
        reader.read_line(&mut response).unwrap();
        println!("{} response: {:?}", command.trim(), response.trim());
        assert_eq!(response, "RPRT -1\n");
    }
    server.stop();

    let before = task_ticks();
    let notifier = DiscordNotifier::new();
    thread::sleep(Duration::from_millis(50));
    let waiting = task_ticks();
    drop(notifier);
    thread::sleep(Duration::from_millis(300));
    let dropped = task_ticks();
    thread::sleep(Duration::from_millis(700));
    let later = task_ticks();
    println!(
        "Discord threads: before={}, alive={}, after drop={}, 700ms later={}",
        before.len(),
        waiting.len(),
        dropped.len(),
        later.len()
    );
    let mut leaked_cpu = 0;
    for (id, ticks) in &later {
        if !before.iter().any(|(old, _)| old == id) {
            let previous = dropped
                .iter()
                .find(|(old, _)| old == id)
                .map_or(0, |(_, t)| *t);
            let delta = ticks.saturating_sub(previous);
            leaked_cpu += delta;
            println!("Remaining Discord task {id}: CPU tick delta={delta}");
        }
    }
    assert!(
        later.len() > before.len(),
        "dropped notifier unexpectedly cleaned up its worker"
    );
    assert!(
        leaked_cpu > 0,
        "expected busy-loop worker consumed no CPU ticks"
    );
}
