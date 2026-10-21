use crate::http::HTTP_VERSION;
use crate::http::headers::Headers;
use crate::http::request::RequestError;
use crate::http::status::StatusCode;

pub struct Response {
    status: StatusCode,
    headers: Headers,
    body: Vec<u8>,
}

impl Response {
    pub fn new(status: StatusCode, mut headers: Headers, body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        headers.set("Content-Length", &body.len().to_string());
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NotFound, Headers::default(), vec![])
    }

    pub fn bad_request() -> Self {
        Self::new(StatusCode::BadRequest, Headers::default(), vec![])
    }

    pub fn status(&self) -> &StatusCode {
        &self.status
    }

    #[cfg(test)]
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(key)
    }

    #[cfg(test)]
    pub fn body(&self) -> &str {
        std::str::from_utf8(&self.body).expect("body is not valid UTF-8")
    }

    pub fn into_bytes(self) -> Vec<u8> {
        let head = format!("{} {}\r\n{}\r\n", HTTP_VERSION, self.status, self.headers);

        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

impl From<RequestError> for Response {
    fn from(_: RequestError) -> Self {
        Response::bad_request()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn empty(status: StatusCode) -> Response {
        Response::new(status, Headers::default(), vec![])
    }

    // Compare as text so assertion failures show readable `\r\n` instead of byte arrays.
    fn serialized(response: Response) -> String {
        String::from_utf8(response.into_bytes()).unwrap()
    }

    #[test]
    fn exposes_status() {
        assert_eq!(empty(StatusCode::NotFound).status(), &StatusCode::NotFound);
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

    fn text_plain(body: &str) -> Response {
        let mut headers = Headers::default();

        headers.add("Content-Type", "text/plain");

        Response::new(StatusCode::Ok, headers, body.as_bytes())
    }

    #[test]
    fn exposes_body() {
        assert_eq!(text_plain("abc").body(), "abc");
    }

    #[test]
    fn exposes_header_value() {
        assert_eq!(text_plain("abc").header("Content-Type"), Some("text/plain"));
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    #[test]
    fn header_lookup_is_case_insensitive() {
        assert_eq!(text_plain("abc").header("content-length"), Some("3"));
    }

    #[test]
    fn missing_header_is_none() {
        assert_eq!(text_plain("abc").header("X-Unknown"), None);
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

        assert_eq!(response.header("Content-Length"), Some("3"));
        assert_eq!(
            serialized(response),
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc"
        );
    }

    // RFC 9110 §8.6: Content-Length is a number of octets, not characters.
    #[test]
    fn content_length_counts_bytes_not_chars() {
        assert_eq!(text_plain("é").header("Content-Length"), Some("2"));
    }

    #[test]
    fn serializing_twice_gives_same_output() {
        let first = text_plain("abc").into_bytes();
        let second = text_plain("abc").into_bytes();

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
        assert_eq!(binary().header("Content-Length"), Some("4"));
    }

    // Compared as bytes: the output is not valid UTF-8, so `serialized` would panic.
    #[test]
    fn serializes_binary_body_unchanged() {
        let mut expected = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n".to_vec();
        expected.extend_from_slice(&BINARY);

        assert_eq!(binary().into_bytes(), expected);
    }

    // RFC 9112 §6.3: the announced length must match the bytes actually sent.
    #[test]
    fn sent_body_length_matches_content_length() {
        let bytes = binary().into_bytes();
        let body_start = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;

        assert_eq!(bytes.len() - body_start, 4);
    }
}
