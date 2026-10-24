use std::str::FromStr;

use std::{
    io::{BufRead, BufReader},
    net::TcpStream,
};

use crate::config::Config;
use crate::handlers::route;
use crate::http::response::Response;
use std::io::Write;

use crate::http::request::{Request, RequestError};

fn read_request(reader: &mut impl BufRead) -> Result<Option<Request>, RequestError> {
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

    let mut request = Request::from_str(&head)?;

    // Body bytes may already sit in the reader's buffer: read them from the reader, never the raw stream.
    let mut body = vec![0u8; request.content_length()?];

    // read body
    reader
        .read_exact(&mut body)
        .map_err(|err| RequestError::Io(err.to_string()))?;

    let body = String::from_utf8(body).map_err(|_| RequestError::InvalidBody)?;

    request.set_body(body);

    Ok(Some(request))
}

pub fn handle_connection(stream: TcpStream, config: &Config) {
    let mut buffer = BufReader::new(stream);

    loop {
        let mut should_drop = false;
        let response: Response = match read_request(&mut buffer) {
            Ok(Some(request)) => {
                let mut response = route(&request, config);

                response.set_encoding(request.accept_encoding());

                response
            }
            Ok(None) => {
                break;
            }
            Err(err) => {
                eprintln!("error: {err}");
                should_drop = true;
                err.into()
            }
        };
        let stream = buffer.get_mut();
        let result = stream.write_all(&response.into_bytes());
        if let Err(e) = result {
            eprintln!("error: {}", e);
            break;
        }
        if should_drop {
            break;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Cursor;

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
}
