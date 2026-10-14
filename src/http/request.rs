use std::str::FromStr;

use crate::http::method::Method;

pub struct Request {
    path: String,
    method: Method,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum RequestError {
    #[error("Invalid method")]
    InvalidMethod,
    #[error("Cannot Parse")]
    CannotParse,
}

impl Request {
    pub fn new(path: &str, method: &str) -> Result<Self, RequestError> {
        let Ok(parsed_method) = Method::try_from(method) else {
            return Err(RequestError::InvalidMethod);
        };

        Ok(Self {
            path: path.into(),
            method: parsed_method,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn method(&self) -> &Method {
        &self.method
    }
}

impl FromStr for Request {
    type Err = RequestError;
    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let mut parts = line.split_whitespace();

        let (Some(method), Some(path), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(RequestError::CannotParse);
        };

        Request::new(path, method)
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
        let request = Request::new("/echo/abc", "GET").unwrap();
        assert_eq!(request.path(), "/echo/abc");
    }

    #[test]
    fn new_rejects_unknown_method() {
        let result = Request::new("/", "DELETE");
        assert_eq!(result.err(), Some(RequestError::InvalidMethod));
    }
}
