use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Write;

use crate::http::headers::Headers;
use crate::http::headers::encoding::Encoding;
use crate::http::request::RequestError;
use crate::http::status::StatusCode;

pub struct Response {
    status: StatusCode,
    headers: Headers,
    body: Vec<u8>,
}

impl Response {
    pub fn new(status: StatusCode, headers: Headers, body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
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

    #[cfg(test)]
    pub fn status(&self) -> &StatusCode {
        &self.status
    }

    #[cfg(test)]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn into_parts(self) -> (StatusCode, Headers, Vec<u8>) {
        (self.status, self.headers, self.body)
    }

    pub fn set_encoding(&mut self, encoding: Encoding) {
        match encoding {
            Encoding::Gzip => {
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(&self.body).unwrap();

                match encoder.finish() {
                    Ok(result) => self.body = result,
                    Err(error) => {
                        eprintln!("error: {}", error);

                        return;
                    }
                }

                self.headers.set("Content-Encoding", Encoding::Gzip.into());
            }
        }
    }

    #[cfg(test)]
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(key)
    }

    #[cfg(test)]
    pub fn body_as_str(&self) -> &str {
        std::str::from_utf8(&self.body).expect("body is not valid UTF-8")
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

    #[test]
    fn exposes_status() {
        assert_eq!(empty(StatusCode::NotFound).status(), &StatusCode::NotFound);
    }

    fn text_plain(body: &str) -> Response {
        let mut headers = Headers::default();

        headers.add("Content-Type", "text/plain");

        Response::new(StatusCode::Ok, headers, body.as_bytes())
    }

    #[test]
    fn exposes_body() {
        assert_eq!(text_plain("abc").body_as_str(), "abc");
    }

    #[test]
    fn exposes_header_value() {
        assert_eq!(text_plain("abc").header("Content-Type"), Some("text/plain"));
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    #[test]
    fn header_lookup_is_case_insensitive() {
        assert_eq!(text_plain("abc").header("content-type"), Some("text/plain"));
    }

    // Content-Length is framing: the HTTP/1.1 serializer adds it (RFC 9112 §6.2).
    #[test]
    fn new_does_not_set_content_length() {
        assert_eq!(text_plain("abc").header("Content-Length"), None);
    }

    #[test]
    fn missing_header_is_none() {
        assert_eq!(text_plain("abc").header("X-Unknown"), None);
    }

    // The conversion only maps an error to a status: closing the connection
    // (and announcing it) is decided by the server loop, in one place.
    #[test]
    fn request_error_converts_to_plain_bad_request() {
        let response: Response = RequestError::TruncatedRequest.into();

        assert_eq!(response.status(), &StatusCode::BadRequest);
        assert_eq!(response.header("Connection"), None);
    }
}
