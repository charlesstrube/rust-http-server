//! Functional tests: spawn the real binary and talk to it over TCP.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
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
        Self::start_with_args(&[])
    }

    fn start_with_args(args: &[&str]) -> Self {
        let lock = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        assert!(
            TcpStream::connect(ADDR).is_err(),
            "port 4221 already in use: stop your running server first"
        );

        let child = Command::new(env!("CARGO_BIN_EXE_codecrafters-http-server"))
            .args(args)
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

fn connect() -> TcpStream {
    let stream = TcpStream::connect(ADDR).expect("failed to connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
}

// Writes `raw` on an already-open connection and reads until the server closes it.
fn send_on(mut stream: TcpStream, raw: &str) -> String {
    stream.write_all(raw.as_bytes()).unwrap();

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("no response in time: is another connection blocking the server?");
    String::from_utf8(response).expect("response is not valid UTF-8")
}

fn send(raw: &str) -> String {
    send_on(connect(), raw)
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
    assert_eq!(
        get("/abcdefg"),
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
    );
}

#[test]
fn request_line_only_is_enough() {
    let _server = Server::start();
    assert_eq!(
        send("GET / HTTP/1.1\r\n\r\n"),
        "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
    );
}

#[test]
fn handles_sequential_connections() {
    let _server = Server::start();
    for _ in 0..5 {
        assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(
            get("/nope"),
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
        );
    }
}

#[test]
fn malformed_request_responds_400() {
    let _server = Server::start();
    assert_eq!(
        send("garbage\r\n\r\n"),
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n"
    );
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn unknown_method_responds_400() {
    let _server = Server::start();
    assert_eq!(
        send("BREW / HTTP/1.1\r\n\r\n"),
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n"
    );
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
    assert_eq!(
        get("/echo"),
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
    );
}

#[test]
fn echo_with_empty_string_responds_404() {
    let _server = Server::start();
    assert_eq!(
        get("/echo/"),
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
    );
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
    for agent in [
        "curl/8.4.0",
        "grape/mango-banana",
        "Mozilla/5.0 (X11; Linux x86_64)",
    ] {
        assert_eq!(
            get_with_headers(
                "/user-agent",
                &format!("Host: localhost:4221\r\nUser-Agent: {agent}\r\n")
            ),
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
        get_with_headers(
            "/user-agent",
            "User-Agent: first/1.0\r\nHost: localhost:4221\r\nAccept: */*\r\n"
        ),
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
    assert_eq!(
        get_with_headers("/user-agent", "User-Agent: a/1\r\n"),
        text_plain_200("a/1")
    );
    assert_eq!(get("/echo/abc"), text_plain_200("abc"));
    assert_eq!(
        get_with_headers("/user-agent", "User-Agent: b/2\r\n"),
        text_plain_200("b/2")
    );
}

#[test]
fn invalid_header_responds_400() {
    let _server = Server::start();
    assert_eq!(
        send("GET / HTTP/1.1\r\nHost : localhost\r\n\r\n"),
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n"
    );
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
}

// --- Concurrent connections ---

const OK_EMPTY: &str = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";

#[test]
fn idle_connection_does_not_block_others() {
    let _server = Server::start();
    let _idle = connect();
    assert_eq!(send("GET / HTTP/1.1\r\n\r\n"), OK_EMPTY);
}

#[test]
fn partial_request_does_not_block_others() {
    let _server = Server::start();
    let mut slow = connect();
    slow.write_all(b"GET /echo/slow HT").unwrap();

    assert_eq!(get("/echo/fast"), text_plain_200("fast"));

    assert_eq!(send_on(slow, "TP/1.1\r\n\r\n"), text_plain_200("slow"));
}

#[test]
fn connections_served_in_reverse_order() {
    let _server = Server::start();
    let streams: Vec<TcpStream> = (0..5).map(|_| connect()).collect();

    for (i, stream) in streams.into_iter().enumerate().rev() {
        assert_eq!(
            send_on(stream, &format!("GET /echo/{i} HTTP/1.1\r\n\r\n")),
            text_plain_200(&i.to_string())
        );
    }
}

// Mirrors the CodeCrafters tester: open every connection first, then send all requests at once.
#[test]
fn many_simultaneous_connections() {
    let _server = Server::start();
    const N: usize = 10;
    let barrier = Arc::new(Barrier::new(N));

    let handles: Vec<_> = (0..N)
        .map(|i| {
            let stream = connect();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let raw = if i % 2 == 0 {
                    "GET / HTTP/1.1\r\n\r\n".to_string()
                } else {
                    format!("GET /echo/{i} HTTP/1.1\r\n\r\n")
                };
                (i, send_on(stream, &raw))
            })
        })
        .collect();

    for handle in handles {
        let (i, response) = handle.join().expect("client thread panicked");
        if i % 2 == 0 {
            assert_eq!(response, OK_EMPTY);
        } else {
            assert_eq!(response, text_plain_200(&i.to_string()));
        }
    }
}

#[test]
fn abandoned_connection_does_not_break_server() {
    let _server = Server::start();
    {
        let mut gone = connect();
        gone.write_all(b"GET / HT").unwrap();
    }
    assert_eq!(get("/"), OK_EMPTY);
}

// --- Return a file ---

// Unique directory under the system temp dir, removed on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("http-server-it-{}-{name}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: &str, contents: &str) {
        fs::write(self.path.join(name), contents).unwrap();
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path.join(name)).unwrap()
    }

    // Mirrors `--directory /tmp/`: the tester passes the directory with a trailing slash.
    fn arg(&self) -> String {
        format!("{}/", self.path.display())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

const NOT_FOUND_EMPTY: &str = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";

fn octet_stream_200(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn files_matches_spec_example() {
    let dir = TempDir::new("spec");
    dir.write("foo", "Hello, World!");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(get("/files/foo"), octet_stream_200("Hello, World!"));
}

#[test]
fn files_missing_responds_404() {
    let dir = TempDir::new("missing");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(get("/files/non_existant_file"), NOT_FOUND_EMPTY);
}

#[test]
fn files_directory_without_trailing_slash() {
    let dir = TempDir::new("no-slash");
    dir.write("bar", "no slash");
    let path = dir.path.display().to_string();
    let _server = Server::start_with_args(&["--directory", &path]);

    assert_eq!(get("/files/bar"), octet_stream_200("no slash"));
}

#[test]
fn files_served_with_random_names_and_contents() {
    let dir = TempDir::new("random");
    let files = [
        ("apple_banana", "mango grape"),
        ("orange-123", "pineapple\nstrawberry"),
        ("x", ""),
    ];
    for (name, contents) in files {
        dir.write(name, contents);
    }
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    for (name, contents) in files {
        assert_eq!(get(&format!("/files/{name}")), octet_stream_200(contents));
    }
}

#[test]
fn files_without_directory_flag_responds_404() {
    let _server = Server::start();

    assert_eq!(get("/files/foo"), NOT_FOUND_EMPTY);
}

#[test]
fn files_and_other_routes_on_same_server() {
    let dir = TempDir::new("mixed");
    dir.write("foo", "Hello, World!");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(get("/files/foo"), octet_stream_200("Hello, World!"));
    assert_eq!(get("/echo/abc"), text_plain_200("abc"));
    assert_eq!(get("/"), OK_EMPTY);
}

// --- Read request body ---

const CREATED_EMPTY: &str = "HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n";
const BAD_REQUEST_EMPTY: &str = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";

// Same request as `curl -v --data "<body>" -H "Content-Type: application/octet-stream"`.
fn post(path: &str, body: &str) -> String {
    send(&format!(
        "POST {path} HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: curl/8.0\r\nAccept: */*\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ))
}

// Sends each part in its own TCP write, with a pause in between,
// so the server cannot receive the whole request in one read.
fn send_in_parts(parts: &[&str]) -> String {
    let mut stream = connect();
    let (last, first) = parts.split_last().unwrap();
    for part in first {
        stream.write_all(part.as_bytes()).unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(100));
    }
    send_on(stream, last)
}

// The client never closes its write side (like curl): reading the body
// until EOF instead of Content-Length bytes makes these tests time out.

#[test]
fn post_file_matches_spec_example() {
    let dir = TempDir::new("post-spec");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(post("/files/file_123", "12345"), CREATED_EMPTY);
    assert_eq!(dir.read("file_123"), "12345");
}

#[test]
fn post_file_with_random_names_and_contents() {
    let dir = TempDir::new("post-random");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);
    let files = [
        ("apple_banana", "mango grape"),
        ("orange-123", "pineapple strawberry raspberry"),
        ("x", "y"),
    ];

    for (name, contents) in files {
        assert_eq!(post(&format!("/files/{name}"), contents), CREATED_EMPTY);
    }
    for (name, contents) in files {
        assert_eq!(dir.read(name), contents);
    }
}

#[test]
fn post_then_get_returns_contents() {
    let dir = TempDir::new("post-get");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(post("/files/round-trip", "there and back"), CREATED_EMPTY);
    assert_eq!(get("/files/round-trip"), octet_stream_200("there and back"));
}

// The body is read by length, not by lines.
#[test]
fn post_body_may_contain_crlf_and_empty_lines() {
    let dir = TempDir::new("post-crlf");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);
    let body = "line 1\r\nline 2\r\n\r\nafter an empty line\n";

    assert_eq!(post("/files/lines", body), CREATED_EMPTY);
    assert_eq!(dir.read("lines"), body);
}

#[test]
fn post_with_empty_body_creates_empty_file() {
    let dir = TempDir::new("post-empty");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(post("/files/empty", ""), CREATED_EMPTY);
    assert_eq!(dir.read("empty"), "");
}

// RFC 9112 §6.3 rule 7: no Content-Length on a request means a zero-length body.
#[test]
fn post_without_content_length_creates_empty_file() {
    let dir = TempDir::new("post-no-length");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(
        send("POST /files/nolength HTTP/1.1\r\nHost: localhost:4221\r\n\r\n"),
        CREATED_EMPTY
    );
    assert_eq!(dir.read("nolength"), "");
}

#[test]
fn post_content_length_name_is_case_insensitive() {
    let dir = TempDir::new("post-case");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(
        send("POST /files/lower HTTP/1.1\r\ncontent-length: 3\r\n\r\nabc"),
        CREATED_EMPTY
    );
    assert_eq!(dir.read("lower"), "abc");
}

// The body arrives in a later TCP segment than the head.
#[test]
fn post_body_sent_after_headers() {
    let dir = TempDir::new("post-split");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(
        send_in_parts(&[
            "POST /files/split HTTP/1.1\r\nContent-Length: 10\r\n\r\n",
            "hello",
            "world",
        ]),
        CREATED_EMPTY
    );
    assert_eq!(dir.read("split"), "helloworld");
}

// Larger than `BufReader`'s default 8 KiB buffer.
#[test]
fn post_large_body() {
    let dir = TempDir::new("post-large");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);
    let body: String = (0..20_000)
        .map(|i| (b'a' + (i % 26) as u8) as char)
        .collect();

    assert_eq!(post("/files/large", &body), CREATED_EMPTY);
    assert_eq!(dir.read("large"), body);
}

// RFC 9112 §6.3 rule 5: an invalid Content-Length is an unrecoverable error.
#[test]
fn post_invalid_content_length_responds_400() {
    let dir = TempDir::new("post-invalid-length");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(
        send("POST /files/bad HTTP/1.1\r\nContent-Length: abc\r\n\r\n"),
        BAD_REQUEST_EMPTY
    );
    assert!(!dir.path.join("bad").exists());
    assert_eq!(get("/"), OK_EMPTY);
}

#[test]
fn post_without_directory_flag_responds_404() {
    let _server = Server::start();

    assert_eq!(post("/files/foo", "12345"), NOT_FOUND_EMPTY);
}

#[test]
fn post_cannot_escape_directory() {
    let dir = TempDir::new("post-traversal");
    let public = dir.path.join("public");
    fs::create_dir_all(&public).unwrap();
    let _server = Server::start_with_args(&["--directory", &format!("{}/", public.display())]);

    assert_eq!(post("/files/../evil", "pwned"), NOT_FOUND_EMPTY);
    assert!(!dir.path.join("evil").exists());
}

#[test]
fn post_and_other_routes_on_same_server() {
    let dir = TempDir::new("post-mixed");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);

    assert_eq!(post("/files/foo", "Hello, World!"), CREATED_EMPTY);
    assert_eq!(get("/echo/abc"), text_plain_200("abc"));
    assert_eq!(get("/files/foo"), octet_stream_200("Hello, World!"));
    assert_eq!(get("/"), OK_EMPTY);
}
