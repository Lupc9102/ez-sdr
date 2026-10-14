#[path = "../../ez-gui/src/rigctl.rs"]
mod rigctl;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
fn main() {
    let mut server = rigctl::RigctlServer::new();
    server.set_enabled(true, 0);
    for _ in 0..100 {
        if server.bound_port() != 0 {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let port = server.bound_port();
    assert_ne!(port, 0);
    let mut first = TcpStream::connect(("127.0.0.1", port)).unwrap();
    first
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    thread::sleep(Duration::from_millis(30));
    let mut second = TcpStream::connect(("127.0.0.1", port)).unwrap();
    second.write_all(b"f\n").unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(
        server.poll_requests().is_empty(),
        "idle first client blocks requests from second client"
    );
    server.set_enabled(true, port);
    let mut b = [0];
    let disconnected = matches!(first.read(&mut b), Ok(0));
    assert!(
        disconnected,
        "same-configuration apply disconnects first client"
    );
    server.stop();
    std::fs::write("audit/2026-10-13/rigctl-lifecycle-result.json", format!("{{\n  \"idle_first_client_blocks_second\": true,\n  \"same_configuration_disconnects_client\": {}\n}}\n",disconnected)).unwrap();
}
