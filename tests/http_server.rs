//! Functional tests: spawn the real binary and talk to it over TCP.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
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

// Reads exactly one response, framed by its Content-Length (RFC 9112 §6.3),
// so it works whether or not the server keeps the connection open afterwards.
fn read_response(reader: &mut impl BufRead) -> RawResponse {
    let mut head = String::new();
    while !head.ends_with("\r\n\r\n") {
        let read = reader
            .read_line(&mut head)
            .expect("no response in time: is another connection blocking the server?");
        assert!(
            read > 0,
            "connection closed before the end of the response headers (got {head:?})"
        );
    }
    head.truncate(head.len() - 4);

    let mut response = RawResponse {
        head,
        body: Vec::new(),
    };
    let length: usize = response
        .header("Content-Length")
        .expect("response has no Content-Length")
        .parse()
        .expect("Content-Length is not a number");

    response.body = vec![0; length];
    reader
        .read_exact(&mut response.body)
        .expect("connection closed before the end of the body");
    response
}

// One TCP connection reused for several requests, like `curl --next`.
struct Connection {
    // Kept for the whole connection: bytes of the next response may already be buffered.
    reader: BufReader<TcpStream>,
}

impl Connection {
    fn open() -> Self {
        Self::from_stream(connect())
    }

    fn from_stream(stream: TcpStream) -> Self {
        Self {
            reader: BufReader::new(stream),
        }
    }

    fn write(&mut self, raw: &str) {
        self.reader.get_mut().write_all(raw.as_bytes()).unwrap();
    }

    fn read_response(&mut self) -> RawResponse {
        read_response(&mut self.reader)
    }

    fn send(&mut self, raw: &str) -> String {
        self.write(raw);
        self.read_response().into_text()
    }

    // Half-close, like a client that has nothing more to send.
    fn shutdown_write(&self) {
        self.reader.get_ref().shutdown(Shutdown::Write).unwrap();
    }

    // Everything the server sends until it closes the connection.
    fn read_until_closed(mut self) -> Vec<u8> {
        let mut rest = Vec::new();
        self.reader
            .read_to_end(&mut rest)
            .expect("server did not close the connection in time");
        rest
    }
}

// Writes `raw` on an already-open connection and reads one response.
fn send_on(stream: TcpStream, raw: &str) -> String {
    Connection::from_stream(stream).send(raw)
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
    assert_eq!(send("garbage\r\n\r\n"), BAD_REQUEST_CLOSE);
    assert_eq!(get("/"), "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
}

#[test]
fn unknown_method_responds_400() {
    let _server = Server::start();
    assert_eq!(send("BREW / HTTP/1.1\r\n\r\n"), BAD_REQUEST_CLOSE);
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
        BAD_REQUEST_CLOSE
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
// RFC 9112 §9.6: a request the server cannot parse ends the connection.
const BAD_REQUEST_CLOSE: &str =
    "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

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
        BAD_REQUEST_CLOSE
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

// --- Compression -----------------------------------------------------------

// A gzip body is not valid UTF-8, so these tests read raw bytes instead of `send`.
struct RawResponse {
    head: String,
    body: Vec<u8>,
}

impl RawResponse {
    fn status_line(&self) -> &str {
        self.head.lines().next().unwrap_or("")
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    fn header(&self, name: &str) -> Option<&str> {
        self.head.split("\r\n").skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    fn into_text(self) -> String {
        let body = String::from_utf8(self.body).expect("response is not valid UTF-8");
        format!("{}\r\n\r\n{body}", self.head)
    }
}

fn get_raw(path: &str, headers: &str) -> RawResponse {
    let mut connection = Connection::open();
    connection.write(&format!(
        "GET {path} HTTP/1.1\r\nHost: localhost:4221\r\n{headers}\r\n"
    ));
    connection.read_response()
}

fn gunzip(bytes: &[u8]) -> String {
    let mut decoded = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut decoded)
        .expect("body is not valid gzip");
    decoded
}

// Stage "Compression headers"

#[test]
fn gzip_accepted_sets_content_encoding() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "Accept-Encoding: gzip\r\n");

    assert_eq!(response.status_line(), "HTTP/1.1 200 OK");
    assert_eq!(response.header("Content-Type"), Some("text/plain"));
    assert_eq!(response.header("Content-Encoding"), Some("gzip"));
}

#[test]
fn unsupported_encoding_has_no_content_encoding() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "Accept-Encoding: invalid-encoding\r\n");

    assert_eq!(response.status_line(), "HTTP/1.1 200 OK");
    assert_eq!(response.header("Content-Type"), Some("text/plain"));
    assert_eq!(response.header("Content-Encoding"), None);
    assert_eq!(response.body, b"abc");
}

#[test]
fn no_accept_encoding_has_no_content_encoding() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "");

    assert_eq!(response.header("Content-Encoding"), None);
    assert_eq!(response.body, b"abc");
}

// Mirrors the tester: curl-like headers, both requests against the same server.
#[test]
fn compression_headers_matches_spec_example() {
    let _server = Server::start();
    const CURL: &str = "User-Agent: curl/7.81.0\r\nAccept: */*\r\n";

    let gzip = get_raw("/echo/abc", &format!("{CURL}Accept-Encoding: gzip\r\n"));
    assert_eq!(gzip.status_line(), "HTTP/1.1 200 OK");
    assert_eq!(gzip.header("Content-Type"), Some("text/plain"));
    assert_eq!(gzip.header("Content-Encoding"), Some("gzip"));

    let invalid = get_raw(
        "/echo/abc",
        &format!("{CURL}Accept-Encoding: invalid-encoding\r\n"),
    );
    assert_eq!(invalid.status_line(), "HTTP/1.1 200 OK");
    assert_eq!(invalid.header("Content-Type"), Some("text/plain"));
    assert_eq!(invalid.header("Content-Encoding"), None);
}

// Stage "Multiple compression schemes" (RFC 9110 §12.5.3: comma-separated list)

#[test]
fn gzip_found_in_list() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "Accept-Encoding: invalid1, gzip, invalid2\r\n");

    assert_eq!(response.header("Content-Encoding"), Some("gzip"));
}

#[test]
fn gzip_found_without_spaces() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "Accept-Encoding: invalid1,gzip\r\n");

    assert_eq!(response.header("Content-Encoding"), Some("gzip"));
}

#[test]
fn list_without_gzip_has_no_content_encoding() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "Accept-Encoding: invalid1, invalid2\r\n");

    assert_eq!(response.header("Content-Encoding"), None);
    assert_eq!(response.body, b"abc");
}

// Beyond CodeCrafters: RFC 9110 §8.4.1, content codings are case-insensitive.
#[test]
fn encoding_name_is_case_insensitive() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "Accept-Encoding: GZip\r\n");

    assert_eq!(response.header("Content-Encoding"), Some("gzip"));
}

// Stage "Gzip compression"

#[test]
fn gzip_body_decompresses_to_original() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "Accept-Encoding: gzip\r\n");

    assert_eq!(gunzip(&response.body), "abc");
}

// RFC 9112 §6.3: Content-Length counts the bytes on the wire, i.e. the compressed ones.
#[test]
fn content_length_matches_compressed_body() {
    let _server = Server::start();
    let response = get_raw("/echo/abc", "Accept-Encoding: gzip\r\n");

    assert_eq!(
        response.header("Content-Length"),
        Some(response.body.len().to_string().as_str())
    );
}

#[test]
fn gzip_and_plain_on_same_server() {
    let _server = Server::start();

    assert_eq!(
        gunzip(&get_raw("/echo/abc", "Accept-Encoding: gzip\r\n").body),
        "abc"
    );
    assert_eq!(get("/echo/abc"), text_plain_200("abc"));
}

// --- Persistent connections -------------------------------------------------
// RFC 9112 §9.3: an HTTP/1.1 connection stays open unless `Connection: close` is sent.

fn curl_get(path: &str, user_agent: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: {user_agent}\r\nAccept: */*\r\n\r\n"
    )
}

// Mirrors `curl --http1.1 http://localhost:4221/echo/banana --next http://localhost:4221/user-agent -H "User-Agent: blueberry/apple-blueberry"`.
#[test]
fn persistent_matches_spec_example() {
    let _server = Server::start();
    let mut connection = Connection::open();

    assert_eq!(
        connection.send(&curl_get("/echo/banana", "curl/8.0")),
        text_plain_200("banana")
    );
    assert_eq!(
        connection.send(&curl_get("/user-agent", "blueberry/apple-blueberry")),
        text_plain_200("blueberry/apple-blueberry")
    );
}

#[test]
fn many_requests_on_same_connection() {
    let _server = Server::start();
    let mut connection = Connection::open();

    for i in 0..5 {
        assert_eq!(
            connection.send(&format!("GET /echo/{i} HTTP/1.1\r\n\r\n")),
            text_plain_200(&i.to_string())
        );
    }
}

// Each request is handled on its own: a 404 does not end the connection.
#[test]
fn mixed_routes_on_same_connection() {
    let _server = Server::start();
    let mut connection = Connection::open();

    assert_eq!(connection.send("GET / HTTP/1.1\r\n\r\n"), OK_EMPTY);
    assert_eq!(
        connection.send("GET /nope HTTP/1.1\r\n\r\n"),
        NOT_FOUND_EMPTY
    );
    assert_eq!(
        connection.send(&curl_get("/user-agent", "a/1")),
        text_plain_200("a/1")
    );
    assert_eq!(
        connection.send("GET /echo/last HTTP/1.1\r\n\r\n"),
        text_plain_200("last")
    );
}

// The client may wait before reusing the connection.
#[test]
fn idle_pause_between_requests() {
    let _server = Server::start();
    let mut connection = Connection::open();

    assert_eq!(connection.send("GET / HTTP/1.1\r\n\r\n"), OK_EMPTY);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        connection.send("GET /echo/later HTTP/1.1\r\n\r\n"),
        text_plain_200("later")
    );
}

// Both requests arrive in a single TCP write (RFC 9112 §9.3.2, pipelining):
// the second one sits in the server's read buffer after the first is parsed.
#[test]
fn pipelined_requests_answered_in_order() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write(&format!(
        "{}{}",
        curl_get("/echo/first", "curl/8.0"),
        curl_get("/echo/second", "curl/8.0")
    ));

    assert_eq!(
        connection.read_response().into_text(),
        text_plain_200("first")
    );
    assert_eq!(
        connection.read_response().into_text(),
        text_plain_200("second")
    );
}

// The body is consumed by Content-Length, so it does not leak into the next request.
#[test]
fn post_then_get_on_same_connection() {
    let dir = TempDir::new("persistent-post");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);
    let mut connection = Connection::open();

    assert_eq!(
        connection.send(
            "POST /files/kept HTTP/1.1\r\nContent-Type: application/octet-stream\r\nContent-Length: 5\r\n\r\n12345"
        ),
        CREATED_EMPTY
    );
    assert_eq!(
        connection.send("GET /files/kept HTTP/1.1\r\n\r\n"),
        octet_stream_200("12345")
    );
}

// Request with a body, then another request, both in one TCP write.
#[test]
fn pipelined_post_then_get() {
    let dir = TempDir::new("persistent-pipelined-post");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);
    let mut connection = Connection::open();

    connection.write(
        "POST /files/piped HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET /echo/next HTTP/1.1\r\n\r\n",
    );

    assert_eq!(connection.read_response().into_text(), CREATED_EMPTY);
    assert_eq!(
        connection.read_response().into_text(),
        text_plain_200("next")
    );
    assert_eq!(dir.read("piped"), "abc");
}

#[test]
fn gzip_then_plain_on_same_connection() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET /echo/abc HTTP/1.1\r\nAccept-Encoding: gzip\r\n\r\n");
    let compressed = connection.read_response();
    assert_eq!(compressed.header("Content-Encoding"), Some("gzip"));
    assert_eq!(gunzip(&compressed.body), "abc");

    assert_eq!(
        connection.send("GET /echo/abc HTTP/1.1\r\n\r\n"),
        text_plain_200("abc")
    );
}

// When the client is done (EOF), the server closes its side instead of
// hanging or answering a phantom request.
#[test]
fn server_closes_when_client_closes() {
    let _server = Server::start();
    let mut connection = Connection::open();

    assert_eq!(connection.send("GET / HTTP/1.1\r\n\r\n"), OK_EMPTY);
    connection.shutdown_write();

    assert_eq!(connection.read_until_closed(), b"");
}

// Same, but without any request: an empty connection gets no response at all.
#[test]
fn connection_closed_without_request_gets_no_response() {
    let _server = Server::start();
    let connection = Connection::open();
    connection.shutdown_write();

    assert_eq!(connection.read_until_closed(), b"");
    assert_eq!(get("/"), OK_EMPTY);
}

// RFC 9112 §8: the client closes before the empty line ending the head,
// so the request is incomplete and must not be routed.
#[test]
fn truncated_request_is_not_routed() {
    let _server = Server::start();
    let mut connection = Connection::open();
    connection.write("GET /echo/abc HTTP/1.1\r\nHost: localhost:4221\r\n");
    connection.shutdown_write();

    assert_eq!(
        String::from_utf8(connection.read_until_closed()).unwrap(),
        BAD_REQUEST_CLOSE
    );
    assert_eq!(get("/"), OK_EMPTY);
}

// A complete request is answered even when a truncated one follows it.
#[test]
fn truncated_request_after_complete_one() {
    let _server = Server::start();
    let mut connection = Connection::open();
    connection.write("GET /echo/first HTTP/1.1\r\n\r\nGET /echo/second HTTP/1.1\r\n");
    connection.shutdown_write();

    assert_eq!(
        connection.read_response().into_text(),
        text_plain_200("first")
    );
    assert_eq!(
        String::from_utf8(connection.read_until_closed()).unwrap(),
        BAD_REQUEST_CLOSE
    );
}

// --- Connection closure -----------------------------------------------------
// RFC 9112 §9.6: after a request with `Connection: close`, the server sends one
// last response carrying `Connection: close`, then closes the connection.

// No client half-close here: `read_until_closed` only returns if the server
// closes on its own (otherwise the read times out and the test fails).

// Mirrors `curl --http1.1 http://localhost:4221/echo/orange --next http://localhost:4221/ -H "Connection: close"`.
#[test]
fn connection_close_matches_spec_example() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write(&curl_get("/echo/orange", "curl/8.0"));
    let first = connection.read_response();
    assert_eq!(first.status_line(), "HTTP/1.1 200 OK");
    assert_eq!(first.body, b"orange");
    assert_eq!(first.header("Connection"), None);

    connection.write(
        "GET / HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: curl/8.0\r\nAccept: */*\r\nConnection: close\r\n\r\n",
    );
    let second = connection.read_response();
    assert_eq!(second.status_line(), "HTTP/1.1 200 OK");
    assert_eq!(second.header("Connection"), Some("close"));

    assert_eq!(connection.read_until_closed(), b"");
}

#[test]
fn close_on_first_request() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET /echo/bye HTTP/1.1\r\nConnection: close\r\n\r\n");
    let response = connection.read_response();
    assert_eq!(response.body, b"bye");
    assert_eq!(response.header("Connection"), Some("close"));

    assert_eq!(connection.read_until_closed(), b"");
}

// Without `Connection: close`, responses must not announce a closure.
#[test]
fn keep_alive_response_has_no_connection_close() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET / HTTP/1.1\r\n\r\n");
    assert_eq!(connection.read_response().header("Connection"), None);
    assert_eq!(connection.send("GET / HTTP/1.1\r\n\r\n"), OK_EMPTY);
}

// RFC 9110 §5.1: field names are case-insensitive.
#[test]
fn connection_header_name_is_case_insensitive() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET / HTTP/1.1\r\nconnection: close\r\n\r\n");
    assert_eq!(
        connection.read_response().header("Connection"),
        Some("close")
    );
    assert_eq!(connection.read_until_closed(), b"");
}

// RFC 9110 §7.6.1: connection options are case-insensitive tokens.
#[test]
fn close_token_is_case_insensitive() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET / HTTP/1.1\r\nConnection: Close\r\n\r\n");
    assert_eq!(
        connection.read_response().header("Connection"),
        Some("close")
    );
    assert_eq!(connection.read_until_closed(), b"");
}

// RFC 9110 §7.6.1: `Connection` is a comma-separated list of options.
#[test]
fn close_token_found_in_list() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET / HTTP/1.1\r\nConnection: keep-alive, close\r\n\r\n");
    assert_eq!(
        connection.read_response().header("Connection"),
        Some("close")
    );
    assert_eq!(connection.read_until_closed(), b"");
}

// Any other option keeps the connection open.
#[test]
fn keep_alive_option_keeps_connection_open() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET / HTTP/1.1\r\nConnection: keep-alive\r\n\r\n");
    assert_ne!(
        connection.read_response().header("Connection"),
        Some("close")
    );
    assert_eq!(
        connection.send("GET /echo/still HTTP/1.1\r\n\r\n"),
        text_plain_200("still")
    );
}

// RFC 9112 §9.6: the server must not process requests received after
// the one carrying `close`, even if they already sit in its read buffer.
#[test]
fn pipelined_request_after_close_is_ignored() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write(
        "GET /echo/last HTTP/1.1\r\nConnection: close\r\n\r\nGET /echo/ignored HTTP/1.1\r\n\r\n",
    );

    let response = connection.read_response();
    assert_eq!(response.body, b"last");
    assert_eq!(response.header("Connection"), Some("close"));
    assert_eq!(connection.read_until_closed(), b"");
}

// Closure does not depend on the route or the status code.
#[test]
fn close_on_not_found() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET /nope HTTP/1.1\r\nConnection: close\r\n\r\n");
    let response = connection.read_response();
    assert_eq!(response.status_line(), "HTTP/1.1 404 Not Found");
    assert_eq!(response.header("Connection"), Some("close"));
    assert_eq!(connection.read_until_closed(), b"");
}

#[test]
fn close_with_gzip_keeps_compression() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection
        .write("GET /echo/zip HTTP/1.1\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n");
    let response = connection.read_response();
    assert_eq!(response.header("Content-Encoding"), Some("gzip"));
    assert_eq!(response.header("Connection"), Some("close"));
    assert_eq!(gunzip(&response.body), "zip");
    assert_eq!(connection.read_until_closed(), b"");
}

#[test]
fn close_after_post_with_body() {
    let dir = TempDir::new("close-post");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);
    let mut connection = Connection::open();

    connection
        .write("POST /files/closed HTTP/1.1\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc");
    let response = connection.read_response();
    assert_eq!(response.status_line(), "HTTP/1.1 201 Created");
    assert_eq!(response.header("Connection"), Some("close"));
    assert_eq!(connection.read_until_closed(), b"");
    assert_eq!(dir.read("closed"), "abc");
}

// Closing one connection must not stop the server from accepting others.
#[test]
fn server_still_accepts_after_close() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("GET / HTTP/1.1\r\nConnection: close\r\n\r\n");
    connection.read_response();
    assert_eq!(connection.read_until_closed(), b"");

    assert_eq!(get("/"), OK_EMPTY);
}

// --- Connection closure on errors -------------------------------------------
// RFC 9112 §9.6: a server that closes after a response SHOULD send `Connection: close`.

// The request cannot be parsed: answer 400, announce the closure, then close.
#[test]
fn parse_error_announces_close_and_closes() {
    let _server = Server::start();
    let mut connection = Connection::open();

    connection.write("garbage\r\n\r\n");
    let response = connection.read_response();
    assert_eq!(response.status_line(), "HTTP/1.1 400 Bad Request");
    assert_eq!(response.header("Connection"), Some("close"));
    assert_eq!(connection.read_until_closed(), b"");
}

// The connection was healthy before the bad request: only the error closes it.
#[test]
fn parse_error_after_valid_request_closes() {
    let _server = Server::start();
    let mut connection = Connection::open();

    assert_eq!(connection.send("GET / HTTP/1.1\r\n\r\n"), OK_EMPTY);
    connection.write("BREW / HTTP/1.1\r\n\r\n");
    assert_eq!(connection.read_response().into_text(), BAD_REQUEST_CLOSE);
    assert_eq!(connection.read_until_closed(), b"");
}

// The framing is lost (unknown body length): the next bytes cannot be trusted.
#[test]
fn invalid_content_length_announces_close_and_closes() {
    let dir = TempDir::new("close-invalid-length");
    let _server = Server::start_with_args(&["--directory", &dir.arg()]);
    let mut connection = Connection::open();

    connection
        .write("POST /files/bad HTTP/1.1\r\nContent-Length: abc\r\n\r\nGET / HTTP/1.1\r\n\r\n");
    assert_eq!(connection.read_response().into_text(), BAD_REQUEST_CLOSE);
    assert_eq!(connection.read_until_closed(), b"");
}

// A 400 produced by a handler is a normal response on a parsed request:
// the connection stays open and nothing announces a closure.
#[test]
fn handler_bad_request_keeps_connection_open() {
    let _server = Server::start();
    let mut connection = Connection::open();

    assert_eq!(
        connection.send("GET /user-agent HTTP/1.1\r\nHost: localhost:4221\r\n\r\n"),
        BAD_REQUEST_EMPTY
    );
    assert_eq!(
        connection.send("GET /echo/still HTTP/1.1\r\n\r\n"),
        text_plain_200("still")
    );
}
