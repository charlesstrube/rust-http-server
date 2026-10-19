use std::str::FromStr;

use crate::http::{
    headers::{Headers, ParseHeadersError},
    method::Method,
};

pub struct Request {
    path: String,
    method: Method,
    headers: Headers,
    body: String,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum RequestError {
    #[error("Invalid method")]
    InvalidMethod,
    #[error("Cannot Parse")]
    CannotParse,
    #[error("Invalid Content Length")]
    InvalidContentLength,
    #[error(transparent)]
    InvalidHeader(#[from] ParseHeadersError),
}

impl Request {
    pub fn new(
        path: &str,
        method: &str,
        headers: Headers,
        body: String,
    ) -> Result<Self, RequestError> {
        let Ok(parsed_method) = Method::try_from(method) else {
            return Err(RequestError::InvalidMethod);
        };

        Ok(Self {
            path: path.into(),
            method: parsed_method,
            headers,
            body,
        })
    }

    pub fn content_length(&self) -> Result<u32, RequestError> {
        match self.header("Content-Length") {
            Some(value) => match value.parse::<u32>() {
                Ok(int) => Ok(int),
                Err(_) => Err(RequestError::InvalidContentLength),
            },
            None => Err(RequestError::InvalidContentLength),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    #[expect(dead_code)]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(key)
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

impl FromStr for Request {
    type Err = RequestError;
    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let (first_line, headers, body) = match line.split_once("\r\n") {
            Some((first_line, header_lines)) => match header_lines.split_once("\r\n\r\n") {
                Some((headers, body)) => {
                    (first_line, headers.parse::<Headers>(), String::from(body))
                }
                None => (first_line, header_lines.parse::<Headers>(), String::new()),
            },
            None => (line, Ok(Headers::default()), String::new()),
        };

        let mut headers = headers?;

        headers.add("Content-Length", &body.len().to_string());

        let mut parts = first_line.split_whitespace();

        let (Some(method), Some(path), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(RequestError::CannotParse);
        };

        Request::new(path, method, headers, body)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_root_path() {
        let request: Request = "GET / HTTP/1.1".parse().unwrap();
        assert_eq!(request.path(), "/");
        assert!(matches!(request.method(), Method::Get));
    }

    #[test]
    fn parses_custom_path() {
        let request: Request = "GET /abcdefg HTTP/1.1".parse().unwrap();
        assert_eq!(request.path(), "/abcdefg");
    }

    #[test]
    fn parses_post() {
        let request: Request = "POST /files/a HTTP/1.1".parse().unwrap();
        assert!(matches!(request.method(), Method::Post));
    }

    // read_line keeps the line terminator.
    #[test]
    fn ignores_trailing_crlf() {
        let request: Request = "GET /index.html HTTP/1.1\r\n".parse().unwrap();
        assert_eq!(request.path(), "/index.html");
    }

    #[test]
    fn rejects_unknown_method() {
        let result = "FOO / HTTP/1.1".parse::<Request>();
        assert_eq!(result.err(), Some(RequestError::InvalidMethod));
    }

    #[test]
    fn rejects_empty_line() {
        let result = "".parse::<Request>();
        assert_eq!(result.err(), Some(RequestError::CannotParse));
    }

    #[test]
    fn rejects_blank_line() {
        let result = "\r\n".parse::<Request>();
        assert_eq!(result.err(), Some(RequestError::CannotParse));
    }

    #[test]
    fn rejects_missing_version() {
        let result = "GET /".parse::<Request>();
        assert_eq!(result.err(), Some(RequestError::CannotParse));
    }

    #[test]
    fn new_builds_request() {
        let request = Request::new("/echo/abc", "GET", Headers::default(), String::new()).unwrap();
        assert_eq!(request.path(), "/echo/abc");
    }

    #[test]
    fn new_rejects_unknown_method() {
        let result = Request::new("/", "DELETE", Headers::default(), String::new());
        assert_eq!(result.err(), Some(RequestError::InvalidMethod));
    }

    // Parsing a full request head: request line, field lines, empty line (RFC 9112 §2.1).

    const SPEC_REQUEST: &str = "GET /user-agent HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: foobar/1.2.3\r\nAccept: */*\r\n\r\n";

    #[test]
    fn parses_request_line_of_full_head() {
        let request: Request = SPEC_REQUEST.parse().unwrap();
        assert_eq!(request.path(), "/user-agent");
        assert!(matches!(request.method(), Method::Get));
    }

    #[test]
    fn parses_headers_after_request_line() {
        let request: Request = SPEC_REQUEST.parse().unwrap();

        assert_eq!(request.header("Host"), Some("localhost:4221"));
        assert_eq!(request.header("User-Agent"), Some("foobar/1.2.3"));
        assert_eq!(request.header("Accept"), Some("*/*"));
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    #[test]
    fn header_lookup_is_case_insensitive() {
        let request: Request = SPEC_REQUEST.parse().unwrap();
        assert_eq!(request.header("user-agent"), Some("foobar/1.2.3"));
        assert_eq!(request.header("USER-AGENT"), Some("foobar/1.2.3"));
    }

    #[test]
    fn missing_header_is_none() {
        let request: Request = SPEC_REQUEST.parse().unwrap();
        assert_eq!(request.header("X-Unknown"), None);
    }

    #[test]
    fn request_line_only_has_no_headers() {
        let request: Request = "GET / HTTP/1.1\r\n\r\n".parse().unwrap();
        assert_eq!(request.header("User-Agent"), None);
    }

    #[test]
    fn new_has_no_headers() {
        let request = Request::new("/", "GET", Headers::default(), String::new()).unwrap();
        assert_eq!(request.header("Host"), None);
    }

    #[test]
    fn rejects_invalid_header_line() {
        let result = "GET / HTTP/1.1\r\ngarbage\r\n\r\n".parse::<Request>();
        assert!(matches!(result, Err(RequestError::InvalidHeader(_))));
    }

    // --- Body (RFC 9112 §6): everything after the empty line closing the head ---

    const SPEC_POST: &str = "POST /files/number HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: curl/7.64.1\r\nAccept: */*\r\nContent-Type: application/octet-stream\r\nContent-Length: 5\r\n\r\n12345";

    #[test]
    fn parses_body_after_empty_line() {
        let request: Request = SPEC_POST.parse().unwrap();
        assert_eq!(request.body(), "12345");
    }

    #[test]
    fn parses_request_line_and_headers_of_request_with_body() {
        let request: Request = SPEC_POST.parse().unwrap();

        assert!(matches!(request.method(), Method::Post));
        assert_eq!(request.path(), "/files/number");
        assert_eq!(
            request.header("Content-Type"),
            Some("application/octet-stream")
        );
        assert_eq!(request.header("Content-Length"), Some("5"));
    }

    // The body is opaque data: it must never reach the header parser.
    #[test]
    fn body_is_not_parsed_as_headers() {
        let request: Request = "POST /files/a HTTP/1.1\r\nContent-Length: 7\r\n\r\ngarbage"
            .parse()
            .unwrap();
        assert_eq!(request.body(), "garbage");
    }

    // Only the first empty line ends the head; later CRLFs belong to the body.
    #[test]
    fn body_may_contain_crlf_and_empty_lines() {
        let request: Request =
            "POST /files/a HTTP/1.1\r\nContent-Length: 10\r\n\r\na\r\nb\r\n\r\nc"
                .parse()
                .unwrap();
        assert_eq!(request.body(), "a\r\nb\r\n\r\nc");
    }

    #[test]
    fn head_without_body_has_empty_body() {
        let request: Request = SPEC_REQUEST.parse().unwrap();
        assert_eq!(request.body(), "");
    }

    #[test]
    fn request_line_only_has_empty_body() {
        let request: Request = "GET / HTTP/1.1".parse().unwrap();
        assert_eq!(request.body(), "");
    }

    #[test]
    fn request_line_with_empty_line_has_empty_body() {
        let request: Request = "GET / HTTP/1.1\r\n\r\n".parse().unwrap();
        assert_eq!(request.body(), "");
    }

    #[test]
    fn new_has_empty_body() {
        let request = Request::new("/", "POST", Headers::default(), String::new()).unwrap();
        assert_eq!(request.body(), "");
    }

    // --- Content-Length (RFC 9110 §8.6, RFC 9112 §6.3) ---

    #[test]
    fn content_length_reads_header_value() {
        let request: Request = SPEC_POST.parse().unwrap();
        assert_eq!(request.content_length(), Ok(5));
    }

    // RFC 9112 §6.3 rule 7: a request without Content-Length (nor Transfer-Encoding) has no body.
    #[test]
    fn missing_content_length_means_zero() {
        let request: Request = SPEC_REQUEST.parse().unwrap();
        assert_eq!(request.content_length(), Ok(0));
    }

    #[test]
    fn content_length_name_is_case_insensitive() {
        let request: Request = "POST /files/a HTTP/1.1\r\ncontent-length: 3\r\n\r\nabc"
            .parse()
            .unwrap();
        assert_eq!(request.content_length(), Ok(3));
    }

    // RFC 9110 §8.6: Content-Length = 1*DIGIT.
    #[test]
    fn rejects_non_numeric_content_length() {
        let request: Request = "POST /files/a HTTP/1.1\r\nContent-Length: abc\r\n\r\n"
            .parse()
            .unwrap();
        assert_eq!(
            request.content_length(),
            Err(RequestError::InvalidContentLength)
        );
    }

    #[test]
    fn rejects_negative_content_length() {
        let request: Request = "POST /files/a HTTP/1.1\r\nContent-Length: -1\r\n\r\n"
            .parse()
            .unwrap();
        assert_eq!(
            request.content_length(),
            Err(RequestError::InvalidContentLength)
        );
    }
}
