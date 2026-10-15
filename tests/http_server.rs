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

// `headers` is a block of `Name: value\r\n` lines; the closing empty line is added here.
fn get_with_headers(path: &str, headers: &str) -> String {
    send(&format!("GET {path} HTTP/1.1\r\n{headers}\r\n"))
}

fn text_plain_200(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn root_responds_200() {
    let _server = Server::start();
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn unknown_path_responds_404() {
    let _server = Server::start();
    assert_eq!(get("/abcdefg"), "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn request_line_only_is_enough() {
    let _server = Server::start();
    assert_eq!(send("GET / HTTP/1.1\r\n\r\n"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn handles_sequential_connections() {
    let _server = Server::start();
    for _ in 0..5 {
        assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(get("/nope"), "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
    }
}

#[test]
fn malformed_request_closes_without_crashing() {
    let _server = Server::start();
    assert_eq!(send("garbage\r\n\r\n"), "");
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn unknown_method_closes_without_crashing() {
    let _server = Server::start();
    assert_eq!(send("BREW / HTTP/1.1\r\n\r\n"), "");
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn echo_matches_spec_example() {
    let _server = Server::start();
    assert_eq!(
        get("/echo/abc"),
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\n\r\nabc"
    );
}

#[test]
fn echo_returns_random_string() {
    let _server = Server::start();
    for word in ["banana", "mango-grape", "x"] {
        assert_eq!(
            get(&format!("/echo/{word}")),
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{word}",
                word.len()
            )
        );
    }
}

#[test]
fn echo_works_with_request_line_only() {
    let _server = Server::start();
    assert_eq!(
        send("GET /echo/hi HTTP/1.1\r\n\r\n"),
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nhi"
    );
}

#[test]
fn echo_without_string_responds_404() {
    let _server = Server::start();
    assert_eq!(get("/echo"), "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn echo_with_empty_string_responds_404() {
    let _server = Server::start();
    assert_eq!(get("/echo/"), "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn echo_and_other_routes_on_same_server() {
    let _server = Server::start();
    assert!(get("/echo/one").ends_with("\r\n\r\none"));
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    assert!(get("/echo/two").ends_with("\r\n\r\ntwo"));
}

#[test]
fn user_agent_matches_spec_example() {
    let _server = Server::start();
    assert_eq!(
        get_with_headers(
            "/user-agent",
            "Host: localhost:4221\r\nUser-Agent: foobar/1.2.3\r\nAccept: */*\r\n"
        ),
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 12\r\n\r\nfoobar/1.2.3"
    );
}

#[test]
fn user_agent_returns_random_values() {
    let _server = Server::start();
    for agent in ["curl/8.4.0", "grape/mango-banana", "Mozilla/5.0 (X11; Linux x86_64)"] {
        assert_eq!(
            get_with_headers("/user-agent", &format!("Host: localhost:4221\r\nUser-Agent: {agent}\r\n")),
            text_plain_200(agent)
        );
    }
}

#[test]
fn user_agent_header_name_is_case_insensitive() {
    let _server = Server::start();
    assert_eq!(
        get_with_headers("/user-agent", "user-agent: foobar/1.2.3\r\n"),
        text_plain_200("foobar/1.2.3")
    );
}

#[test]
fn user_agent_header_position_does_not_matter() {
    let _server = Server::start();
    assert_eq!(
        get_with_headers("/user-agent", "User-Agent: first/1.0\r\nHost: localhost:4221\r\nAccept: */*\r\n"),
        text_plain_200("first/1.0")
    );
}

#[test]
fn user_agent_from_default_request() {
    let _server = Server::start();
    assert_eq!(get("/user-agent"), text_plain_200("curl/8.0"));
}

#[test]
fn user_agent_missing_responds_400() {
    let _server = Server::start();
    assert_eq!(
        get_with_headers("/user-agent", "Host: localhost:4221\r\n"),
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n"
    );
}

#[test]
fn user_agent_and_echo_on_same_server() {
    let _server = Server::start();
    assert_eq!(get_with_headers("/user-agent", "User-Agent: a/1\r\n"), text_plain_200("a/1"));
    assert_eq!(get("/echo/abc"), text_plain_200("abc"));
    assert_eq!(get_with_headers("/user-agent", "User-Agent: b/2\r\n"), text_plain_200("b/2"));
}

#[test]
fn invalid_header_does_not_crash_server() {
    let _server = Server::start();
    let _ = send("GET / HTTP/1.1\r\nHost : localhost\r\n\r\n");
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
}
