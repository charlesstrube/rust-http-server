use std::fmt::Display;

use crate::{
    http::{headers::Headers, response::Response},
    http1::HTTP_VERSION,
};

pub fn serialize_response(response: Response, close: bool) -> Vec<u8> {
    let (status, mut headers, body) = response.into_parts();
    headers.set("Content-Length", &body.len().to_string());
    if close {
        headers.set("Connection", "close");
    } else {
        headers.delete("Connection");
    }
    let head = format!("{} {}\r\n{}\r\n", HTTP_VERSION, status, headers);

    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body.as_slice());
    bytes
}

impl Display for Headers {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (key, value) in self.iter() {
            write!(f, "{}: {}\r\n", key, value)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::{headers::encoding::Encoding, status::StatusCode};

    fn empty(status: StatusCode) -> Response {
        Response::new(status, Headers::default(), vec![])
    }

    fn text_plain(body: &str) -> Response {
        let mut headers = Headers::default();

        headers.add("Content-Type", "text/plain");

        Response::new(StatusCode::Ok, headers, body.as_bytes())
    }

    // Byte-level search: a gzipped or binary body is not valid UTF-8.
    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    // Compare as text so assertion failures show readable `\r\n` instead of byte arrays.
    fn serialized(response: Response) -> String {
        String::from_utf8(serialize_response(response, false)).unwrap()
    }

    // Same, for the last response of a connection (`close = true`).
    fn serialized_closing(response: Response) -> String {
        String::from_utf8(serialize_response(response, true)).unwrap()
    }

    #[test]
    fn serializes_200_status_line() {
        assert_eq!(
            serialized(empty(StatusCode::Ok)),
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn serializes_404_status_line() {
        assert_eq!(
            serialized(empty(StatusCode::NotFound)),
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn ends_with_empty_line() {
        assert!(serialized(empty(StatusCode::Ok)).ends_with("\r\n\r\n"));
    }

    #[test]
    fn serializes_headers_in_insertion_order() {
        assert_eq!(
            serialized(text_plain("")),
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n\r\n"
        );
    }

    // RFC 9112 §6.3: multiple Content-Length values make the message invalid.
    #[test]
    fn content_length_overrides_handler_value() {
        let mut headers = Headers::default();
        headers.add("Content-Length", "999");

        let response = Response::new(StatusCode::Ok, headers, "abc".as_bytes());

        assert_eq!(
            serialized(response),
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc"
        );
    }

    // RFC 9110 §8.6: Content-Length is a number of octets, not characters.
    #[test]
    fn content_length_counts_bytes_not_chars() {
        assert!(serialized(text_plain("é")).contains("\r\nContent-Length: 2\r\n"));
    }

    #[test]
    fn serializing_twice_gives_same_output() {
        let first = serialize_response(text_plain("abc"), false);
        let second = serialize_response(text_plain("abc"), false);

        assert_eq!(String::from_utf8(first), String::from_utf8(second));
    }

    #[test]
    fn serializes_body_after_empty_line() {
        assert_eq!(
            serialized(text_plain("abc")),
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\n\r\nabc"
        );
    }

    // Not valid UTF-8: 0xff never appears in UTF-8, 0x1f 0x8b is the gzip magic number.
    const BINARY: [u8; 4] = [0xff, 0x00, 0x1f, 0x8b];

    fn binary() -> Response {
        Response::new(StatusCode::Ok, Headers::default(), BINARY.to_vec())
    }

    #[test]
    fn content_length_counts_binary_bytes() {
        assert!(contains(
            &serialize_response(binary(), false),
            b"\r\nContent-Length: 4\r\n"
        ));
    }

    // Content-Length is computed at serialization: it must describe the compressed body.
    #[test]
    fn content_length_matches_gzipped_body() {
        let mut response = text_plain("abc");
        response.set_encoding(Encoding::Gzip);
        let expected = format!("\r\nContent-Length: {}\r\n", response.body().len());

        assert!(contains(
            &serialize_response(response, false),
            expected.as_bytes()
        ));
    }

    // Compared as bytes: the output is not valid UTF-8, so `serialized` would panic.
    #[test]
    fn serializes_binary_body_unchanged() {
        let mut expected = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n".to_vec();
        expected.extend_from_slice(&BINARY);

        assert_eq!(serialize_response(binary(), false), expected);
    }

    // RFC 9112 §6.3: the announced length must match the bytes actually sent.
    #[test]
    fn sent_body_length_matches_content_length() {
        let bytes = serialize_response(binary(), false);
        let body_start = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;

        assert_eq!(bytes.len() - body_start, 4);
    }

    // --- Connection closure (RFC 9112 §9.6) ---
    // The serializer owns the `Connection` header: handlers never decide it.

    #[test]
    fn keep_alive_sends_no_connection_header() {
        assert!(!serialized(text_plain("abc")).contains("Connection:"));
    }

    #[test]
    fn close_announces_connection_close() {
        assert!(serialized_closing(text_plain("abc")).contains("\r\nConnection: close\r\n"));
    }

    // Closing only adds one line: status, other headers and body are unchanged.
    #[test]
    fn close_keeps_status_headers_and_body() {
        assert_eq!(
            serialized_closing(text_plain("abc")),
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc"
        );
    }

    // Framing first, then connection management.
    #[test]
    fn close_header_comes_after_content_length() {
        assert_eq!(
            serialized_closing(empty(StatusCode::Ok)),
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    }

    // A `Connection` header set by a handler must not produce a second one.
    #[test]
    fn close_replaces_handler_connection_header() {
        let mut headers = Headers::default();
        headers.add("Connection", "keep-alive");
        let output = serialized_closing(Response::new(StatusCode::Ok, headers, vec![]));

        assert_eq!(output.matches("Connection:").count(), 1);
        assert!(output.contains("\r\nConnection: close\r\n"));
    }

    // Keep-alive is the HTTP/1.1 default (RFC 9112 §9.3): a handler cannot close the connection
    // by announcing it, since the server loop would keep reading anyway.
    #[test]
    fn keep_alive_drops_handler_connection_header() {
        let mut headers = Headers::default();
        headers.add("Connection", "close");

        assert!(
            !serialized(Response::new(StatusCode::Ok, headers, vec![])).contains("Connection:")
        );
    }

    #[test]
    fn close_works_with_gzip_encoding() {
        let mut response = text_plain("abc");
        response.set_encoding(Encoding::Gzip);
        let bytes = serialize_response(response, true);

        assert!(contains(&bytes, b"\r\nContent-Encoding: gzip\r\n"));
        assert!(contains(&bytes, b"\r\nConnection: close\r\n"));
    }

    // A 400 from a handler (e.g. missing User-Agent) keeps the connection usable.
    #[test]
    fn bad_request_does_not_close_by_itself() {
        assert!(!serialized(Response::bad_request()).contains("Connection:"));
    }

    // --- Header section (RFC 9112 §5) ---

    #[test]
    fn displays_one_crlf_terminated_line_per_header() {
        let headers = Headers::from(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.to_string(), "Content-Type: text/plain\r\n");
    }

    #[test]
    fn displays_in_insertion_order() {
        let headers = Headers::from(vec![("B".into(), "2".into()), ("A".into(), "1".into())]);

        assert_eq!(headers.to_string(), "B: 2\r\nA: 1\r\n");
    }

    #[test]
    fn displays_name_with_original_case() {
        let headers = Headers::from(vec![("x-CuStOm".into(), "v".into())]);

        assert_eq!(headers.to_string(), "x-CuStOm: v\r\n");
    }
}
