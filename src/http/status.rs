use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum StatusCode {
    Ok,
    NotFound,
    BadRequest,
}

impl StatusCode {
    fn reason(&self) -> &str {
        match self {
            StatusCode::Ok => "OK",
            StatusCode::NotFound => "Not Found",
            StatusCode::BadRequest => "Bad Request",
        }
    }

    fn code(&self) -> u16 {
        match self {
            StatusCode::Ok => 200,
            StatusCode::NotFound => 404,
            StatusCode::BadRequest => 400,
        }
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.code(), self.reason())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn ok_code_and_reason() {
        assert_eq!(StatusCode::Ok.code(), 200);
        assert_eq!(StatusCode::Ok.reason(), "OK");
    }

    #[test]
    fn not_found_code_and_reason() {
        assert_eq!(StatusCode::NotFound.code(), 404);
        assert_eq!(StatusCode::NotFound.reason(), "Not Found");
    }

    #[test]
    fn bad_request_code_and_reason() {
        assert_eq!(StatusCode::BadRequest.code(), 400);
        assert_eq!(StatusCode::BadRequest.reason(), "Bad Request");
    }

    #[test]
    fn displays_code_then_reason() {
        assert_eq!(StatusCode::Ok.to_string(), "200 OK");
        assert_eq!(StatusCode::NotFound.to_string(), "404 Not Found");
        assert_eq!(StatusCode::BadRequest.to_string(), "400 Bad Request");
    }
}
