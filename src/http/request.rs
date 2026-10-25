use crate::http::{
    headers::{Headers, ParseHeadersError, encoding::Encoding},
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
    #[error("Invalid Body")]
    InvalidBody,
    #[error("Invalid Content Length")]
    InvalidContentLength,
    #[error("Truncated request")]
    TruncatedRequest,
    #[error(transparent)]
    InvalidHeader(#[from] ParseHeadersError),
    #[error("I/O error")]
    Io(String),
}

impl Request {
    pub fn new(
        path: &str,
        method: &str,
        headers: Headers,
        body: &str,
    ) -> Result<Self, RequestError> {
        let Ok(parsed_method) = Method::try_from(method) else {
            return Err(RequestError::InvalidMethod);
        };

        Ok(Self {
            path: path.into(),
            method: parsed_method,
            headers,
            body: body.into(),
        })
    }

    pub fn set_body(&mut self, body: String) {
        self.body = body;
    }

    pub fn accept_encoding(&self) -> Option<Vec<Encoding>> {
        match self.header("Accept-Encoding") {
            Some(headers) => {
                let list: Vec<Encoding> = headers
                    .split(',')
                    .map(str::trim)
                    .filter_map(|coding| coding.parse().ok())
                    .collect();
                if !list.is_empty() { Some(list) } else { None }
            }
            None => None,
        }
    }

    pub fn has_accept_encoding_gzip(&self) -> bool {
        match &self.accept_encoding() {
            Some(headers) => headers.contains(&Encoding::Gzip),
            _ => false,
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

#[cfg(test)]
mod test {
    use super::*;

    fn with_header(name: &str, value: &str) -> Request {
        Request::new(
            "/",
            "GET",
            Headers::from(vec![(name.into(), value.into())]),
            "",
        )
        .unwrap()
    }

    fn bare() -> Request {
        Request::new("/", "GET", Headers::default(), "").unwrap()
    }

    #[test]
    fn new_builds_request() {
        let request = Request::new("/echo/abc", "GET", Headers::default(), "").unwrap();
        assert_eq!(request.path(), "/echo/abc");
    }

    #[test]
    fn new_rejects_unknown_method() {
        let result = Request::new("/", "DELETE", Headers::default(), "");
        assert_eq!(result.err(), Some(RequestError::InvalidMethod));
    }

    #[test]
    fn new_has_no_headers() {
        let request = Request::new("/", "GET", Headers::default(), "").unwrap();
        assert_eq!(request.header("Host"), None);
    }

    #[test]
    fn new_has_empty_body() {
        let request = Request::new("/", "POST", Headers::default(), "").unwrap();
        assert_eq!(request.body(), "");
    }

    // --- Accept-Encoding ---
    // An empty list means "nothing usable": no Option needed on top of the Vec.

    #[test]
    fn accept_encoding_absent_is_empty() {
        let request = bare();
        assert!(request.accept_encoding().is_none());
        assert!(!request.has_accept_encoding_gzip());
    }

    #[test]
    fn accept_encoding_gzip() {
        let request = with_header("Accept-Encoding", "gzip");
        assert!(matches!(
            request.accept_encoding().unwrap().as_slice(),
            [Encoding::Gzip]
        ));
        assert!(request.has_accept_encoding_gzip());
    }

    // RFC 9110 §5.1 and §8.4.1: field name and codings are case-insensitive.
    #[test]
    fn accept_encoding_is_case_insensitive() {
        let request = with_header("accept-encoding", "GZip");
        assert!(request.has_accept_encoding_gzip());
    }

    // RFC 9110 §12.5.3: Accept-Encoding is a comma-separated list.
    #[test]
    fn accept_encoding_finds_gzip_in_list() {
        let request = with_header("Accept-Encoding", "encoding-1, gzip, encoding-2");
        assert!(matches!(
            request.accept_encoding().unwrap().as_slice(),
            [Encoding::Gzip]
        ));
        assert!(request.has_accept_encoding_gzip());
    }

    #[test]
    fn accept_encoding_list_tolerates_spaces() {
        let request = with_header("Accept-Encoding", "br ,  gzip");
        assert!(request.has_accept_encoding_gzip());
    }

    #[test]
    fn accept_encoding_without_supported_coding_is_empty() {
        let request = with_header("Accept-Encoding", "invalid-1, invalid-2");
        assert!(request.accept_encoding().is_none());
        assert!(!request.has_accept_encoding_gzip());
    }
}
