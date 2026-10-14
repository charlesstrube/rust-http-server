//! Functional tests: spawn the real binary and talk to it over TCP.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

const ADDR: &str = "127.0.0.1:4221";

// The port is hardcoded, so tests must not run their servers in parallel.
static PORT_LOCK: Mutex<()> = Mutex::new(());

struct Server {
    child: Child,
    _lock: MutexGuard<'static, ()>,
}

impl Server {
    fn start() -> Self {
        let lock = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        assert!(
            TcpStream::connect(ADDR).is_err(),
            "port 4221 already in use: stop your running server first"
        );

        let child = Command::new(env!("CARGO_BIN_EXE_codecrafters-http-server"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn server");

        let deadline = Instant::now() + Duration::from_secs(5);
        while TcpStream::connect(ADDR).is_err() {
            assert!(Instant::now() < deadline, "server did not start in time");
            thread::sleep(Duration::from_millis(20));
        }

        Self { child, _lock: lock }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn send(raw: &str) -> String {
    let mut stream = TcpStream::connect(ADDR).expect("failed to connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(raw.as_bytes()).unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("failed to read response");
    String::from_utf8(response).expect("response is not valid UTF-8")
}

fn get(path: &str) -> String {
    send(&format!(
        "GET {path} HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: curl/8.0\r\nAccept: */*\r\n\r\n"
    ))
}

#[test]
fn root_responds_200() {
    let _server = Server::start();
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\n\r\n");
}

#[test]
fn unknown_path_responds_404() {
    let _server = Server::start();
    assert_eq!(get("/abcdefg"), "HTTP/1.1 404 Not Found\r\n\r\n");
}

#[test]
fn request_line_only_is_enough() {
    let _server = Server::start();
    assert_eq!(send("GET / HTTP/1.1\r\n\r\n"), "HTTP/1.1 200 OK\r\n\r\n");
}

#[test]
fn handles_sequential_connections() {
    let _server = Server::start();
    for _ in 0..5 {
        assert_eq!(get("/"), "HTTP/1.1 200 OK\r\n\r\n");
        assert_eq!(get("/nope"), "HTTP/1.1 404 Not Found\r\n\r\n");
    }
}

#[test]
fn malformed_request_closes_without_crashing() {
    let _server = Server::start();
    assert_eq!(send("garbage\r\n\r\n"), "");
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\n\r\n");
}

#[test]
fn unknown_method_closes_without_crashing() {
    let _server = Server::start();
    assert_eq!(send("BREW / HTTP/1.1\r\n\r\n"), "");
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\n\r\n");
}
