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
