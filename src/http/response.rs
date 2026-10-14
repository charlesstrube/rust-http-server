use crate::http::HTTP_VERSION;
use crate::http::status::StatusCode;

pub struct Response {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: String,
}

impl Response {
    pub fn new(status: StatusCode, headers: Vec<(String, String)>, body: String) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn status(&self) -> &StatusCode {
        &self.status
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let formated = format!("{} {}\r\n\r\n", HTTP_VERSION, self.status());
        formated.into_bytes()
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
        assert_eq!(empty(StatusCode::Ok).to_bytes(), b"HTTP/1.1 200 OK\r\n\r\n");
    }

    #[test]
    fn serializes_404_status_line() {
        assert_eq!(
            empty(StatusCode::NotFound).to_bytes(),
            b"HTTP/1.1 404 Not Found\r\n\r\n"
        );
    }

    #[test]
    fn ends_with_empty_line() {
        assert!(empty(StatusCode::Ok).to_bytes().ends_with(b"\r\n\r\n"));
    }
}
