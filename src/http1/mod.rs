pub mod connection;
pub mod parse;
pub mod serialize;

pub const HTTP_VERSION: &str = "HTTP/1.1";

use std::io::BufRead;
use std::io::Write;

use crate::http::response::Response;
use crate::http1::connection::wants_close;
use crate::http1::parse::content_length;
use crate::http1::parse::parse_request;

use crate::http::request::{Request, RequestError};
use crate::http1::serialize::serialize_response;

pub fn read_request(reader: &mut impl BufRead) -> Result<Option<Request>, RequestError> {
    let mut head = String::new();
    loop {
        let read = reader
            .read_line(&mut head)
            .map_err(|err| RequestError::Io(err.to_string()))?;

        // if there's no additional request in the reader
        if read == 0 && head.is_empty() {
            return Ok(None);
        }

        // EOF mid-head: the request is truncated (head is not empty here)
        if read == 0 {
            return Err(RequestError::TruncatedRequest);
        }

        // Empty line reached: the head is complete, the body (if any) follows.
        if head.ends_with("\r\n\r\n") {
            break;
        }
    }

    let mut request = parse_request(&head)?;

    // Body bytes may already sit in the reader's buffer: read them from the reader, never the raw stream.
    let mut body = vec![0u8; content_length(&request)?];

    // read body
    reader
        .read_exact(&mut body)
        .map_err(|err| RequestError::Io(err.to_string()))?;

    let body = String::from_utf8(body).map_err(|_| RequestError::InvalidBody)?;

    request.set_body(body);

    Ok(Some(request))
}

pub fn serve(
    mut reader: impl BufRead,
    mut writer: impl Write,
    handler: impl Fn(&Request) -> Response,
) {
    loop {
        let (response, should_close) = match read_request(&mut reader) {
            Ok(Some(request)) => (handler(&request), wants_close(&request)),
            Ok(None) => {
                break;
            }
            Err(err) => {
                eprintln!("error: {err}");
                (err.into(), true)
            }
        };

        let serialized_response = serialize_response(response, should_close);

        let result = writer.write_all(serialized_response.as_slice());
        if let Err(e) = result {
            eprintln!("error: {}", e);
            break;
        }
        if should_close {
            break;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::{headers::Headers, status::StatusCode};
    use std::cell::Cell;
    use std::io::{self, Cursor};

    // A persistent connection reads every request from the same reader:
    // whatever follows a request must stay available for the next call.
    #[test]
    fn reads_pipelined_requests_from_same_reader() {
        let mut reader = Cursor::new(
            "GET /echo/banana HTTP/1.1\r\n\r\nGET /user-agent HTTP/1.1\r\nUser-Agent: blueberry\r\n\r\n",
        );

        let first = read_request(&mut reader).unwrap().unwrap();
        let second = read_request(&mut reader).unwrap().unwrap();

        assert_eq!(first.path(), "/echo/banana");
        assert_eq!(second.path(), "/user-agent");
        assert_eq!(second.header("User-Agent"), Some("blueberry"));
    }

    #[test]
    fn body_stops_at_content_length() {
        let mut reader = Cursor::new(
            "POST /files/a HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET / HTTP/1.1\r\n\r\n",
        );

        let post = read_request(&mut reader).unwrap().unwrap();
        let get = read_request(&mut reader).unwrap().unwrap();

        assert_eq!(post.body(), "abc");
        assert_eq!(get.path(), "/");
        assert_eq!(get.body(), "");
    }

    // EOF before any byte is how a persistent connection normally ends.
    #[test]
    fn eof_before_any_byte_is_none() {
        let mut reader = Cursor::new("");

        assert!(matches!(read_request(&mut reader), Ok(None)));
    }

    #[test]
    fn eof_after_last_request_is_none() {
        let mut reader = Cursor::new("GET / HTTP/1.1\r\n\r\n");

        assert!(matches!(read_request(&mut reader), Ok(Some(_))));
        assert!(matches!(read_request(&mut reader), Ok(None)));
    }

    // RFC 9112 §8: a message cut off by the connection closing is incomplete.
    #[test]
    fn eof_after_request_line_is_truncated() {
        let mut reader = Cursor::new("GET /echo/abc HTTP/1.1\r\n");

        assert!(matches!(
            read_request(&mut reader),
            Err(RequestError::TruncatedRequest)
        ));
    }

    #[test]
    fn eof_in_the_middle_of_headers_is_truncated() {
        let mut reader = Cursor::new("GET /echo/abc HTTP/1.1\r\nHost: localhost\r\nUser-Ag");

        assert!(matches!(
            read_request(&mut reader),
            Err(RequestError::TruncatedRequest)
        ));
    }

    #[test]
    fn eof_before_empty_line_is_truncated() {
        let mut reader = Cursor::new("GET /echo/abc HTTP/1.1\r\nHost: localhost\r\n");

        assert!(matches!(
            read_request(&mut reader),
            Err(RequestError::TruncatedRequest)
        ));
    }

    // --- serve: the HTTP/1.1 connection loop (RFC 9112 §9) ---

    // Answers with the request path as body: shows which request produced which response.
    fn echo_path(request: &Request) -> Response {
        Response::new(StatusCode::Ok, Headers::default(), request.path())
    }

    // Runs `serve` over an in-memory connection and returns everything it wrote.
    fn run(input: &str, handler: impl Fn(&Request) -> Response) -> String {
        let mut output = Vec::new();
        serve(Cursor::new(input), &mut output, handler);
        // Lossy: a wrongly gzipped body must fail the assertion, not panic here.
        String::from_utf8_lossy(&output).into_owned()
    }

    // Same, but also counts handler calls (`Fn` allows mutation through a `Cell`).
    fn run_counting(input: &str) -> (String, usize) {
        let calls = Cell::new(0);
        let output = run(input, |request| {
            calls.set(calls.get() + 1);
            echo_path(request)
        });
        (output, calls.get())
    }

    const BAD_REQUEST_AND_CLOSE: &str =
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    #[test]
    fn answers_a_single_request() {
        assert_eq!(
            run("GET /a HTTP/1.1\r\n\r\n", echo_path),
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n/a"
        );
    }

    // EOF before any byte is the normal end of a persistent connection: nothing to answer.
    #[test]
    fn writes_nothing_when_client_sends_nothing() {
        assert_eq!(run_counting(""), (String::new(), 0));
    }

    // RFC 9112 §9.3: connections are persistent by default.
    #[test]
    fn answers_every_request_of_a_persistent_connection() {
        assert_eq!(
            run("GET /a HTTP/1.1\r\n\r\nGET /b HTTP/1.1\r\n\r\n", echo_path),
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n/a\
             HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n/b"
        );
    }

    #[test]
    fn calls_handler_once_per_request() {
        let (_, calls) =
            run_counting("GET /a HTTP/1.1\r\n\r\nGET /b HTTP/1.1\r\n\r\nGET /c HTTP/1.1\r\n\r\n");
        assert_eq!(calls, 3);
    }

    // RFC 9112 §9.6: after `Connection: close`, the server answers then stops reading.
    #[test]
    fn connection_close_ends_after_its_response() {
        let (output, calls) =
            run_counting("GET /a HTTP/1.1\r\nConnection: close\r\n\r\nGET /b HTTP/1.1\r\n\r\n");

        assert_eq!(
            output,
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n/a"
        );
        assert_eq!(calls, 1);
    }

    // The request body is consumed before the next request is read.
    #[test]
    fn request_body_does_not_leak_into_next_request() {
        let (output, calls) =
            run_counting("POST /a HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET /b HTTP/1.1\r\n\r\n");

        assert!(output.ends_with("\r\n\r\n/b"));
        assert_eq!(calls, 2);
    }

    // A request that cannot be parsed is never routed: 400, announced close, then stop.
    #[test]
    fn parse_error_answers_400_and_closes() {
        let (output, calls) = run_counting("FOO / HTTP/1.1\r\n\r\nGET /b HTTP/1.1\r\n\r\n");

        assert_eq!(output, BAD_REQUEST_AND_CLOSE);
        assert_eq!(calls, 0);
    }

    // RFC 9112 §8: a request cut off by EOF is incomplete, so it must not be routed.
    #[test]
    fn truncated_request_answers_400_and_closes() {
        let (output, calls) = run_counting("GET /a HTTP/1.1\r\nHost: localhost\r\n");

        assert_eq!(output, BAD_REQUEST_AND_CLOSE);
        assert_eq!(calls, 0);
    }

    #[test]
    fn parse_error_after_valid_request_keeps_first_response() {
        let (output, calls) = run_counting("GET /a HTTP/1.1\r\n\r\nFOO / HTTP/1.1\r\n\r\n");

        assert_eq!(
            output,
            format!("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n/a{BAD_REQUEST_AND_CLOSE}")
        );
        assert_eq!(calls, 1);
    }

    // A 400 decided by a handler (e.g. missing User-Agent) is a normal answer: the connection stays open.
    #[test]
    fn handler_400_keeps_connection_open() {
        let output = run(
            "GET /bad HTTP/1.1\r\n\r\nGET /b HTTP/1.1\r\n\r\n",
            |request| match request.path() {
                "/bad" => Response::bad_request(),
                _ => echo_path(request),
            },
        );

        assert_eq!(
            output,
            "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n\
             HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n/b"
        );
    }

    // Compression is not HTTP/1.1 framing: it belongs to the shared handler, so HTTP/2 gets it too.
    #[test]
    fn does_not_compress_by_itself() {
        let output = run(
            "GET /a HTTP/1.1\r\nAccept-Encoding: gzip\r\n\r\n",
            echo_path,
        );

        assert_eq!(output, "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n/a");
    }

    // Writes always fail, like a socket the client has closed.
    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    // Once the client is gone, the remaining requests are not processed.
    #[test]
    fn stops_when_writing_fails() {
        let calls = Cell::new(0);
        serve(
            Cursor::new("GET /a HTTP/1.1\r\n\r\nGET /b HTTP/1.1\r\n\r\n"),
            BrokenPipe,
            |request| {
                calls.set(calls.get() + 1);
                echo_path(request)
            },
        );

        assert_eq!(calls.get(), 1);
    }
}
