use crate::http::HTTP_VERSION;
use crate::http::status::StatusCode;

pub struct Response {
    status: StatusCode,
    header: Vec<(String, String)>,
    body: String,
}

impl Response {
    pub fn new(status: StatusCode, header: Vec<(String, String)>, body: String) -> Self {
        Self {
            status,
            header,
            body,
        }
    }

    pub fn status(&self) -> &StatusCode {
        &self.status
    }

    pub fn to_write(&self) -> Vec<u8> {
        let content_type = match self.header("content-type") {
            Some(value) => format!("\r\nContent-Type: {}", value),
            None => String::new(),
        };

        let content_length = match self.header("content-length") {
            Some(value) => format!("\r\nContent-Length: {}", value),
            None => String::new(),
        };

        let formated = format!(
            "{} {}{}{}\r\n\r\n{}",
            HTTP_VERSION,
            self.status(),
            content_type,
            content_length,
            self.body
        );

        formated.into_bytes()
    }

    pub fn header(&self, needle: &str) -> Option<&str> {
        self.header.iter().find_map(|(key, value)| {
            if key.eq_ignore_ascii_case(needle) {
                Some(value.as_str())
            } else {
                None
            }
        })
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn empty(status: StatusCode) -> Response {
        Response::new(status, vec![], String::new())
    }

    #[test]
    fn exposes_status() {
        assert_eq!(empty(StatusCode::NotFound).status(), &StatusCode::NotFound);
    }

    #[test]
    fn serializes_200_status_line() {
        assert_eq!(empty(StatusCode::Ok).to_write(), b"HTTP/1.1 200 OK\r\n\r\n");
    }

    #[test]
    fn serializes_404_status_line() {
        assert_eq!(
            empty(StatusCode::NotFound).to_write(),
            b"HTTP/1.1 404 Not Found\r\n\r\n"
        );
    }

    #[test]
    fn ends_with_empty_line() {
        assert!(empty(StatusCode::Ok).to_write().ends_with(b"\r\n\r\n"));
    }

    fn text_plain(body: &str) -> Response {
        Response::new(
            StatusCode::Ok,
            vec![
                ("Content-Type".into(), "text/plain".into()),
                ("Content-Length".into(), body.len().to_string()),
            ],
            body.into(),
        )
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
            text_plain("").to_write(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn serializes_body_after_empty_line() {
        assert_eq!(
            text_plain("abc").to_write(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\n\r\nabc"
        );
    }
}
