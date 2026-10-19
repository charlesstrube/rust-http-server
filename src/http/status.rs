use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum StatusCode {
    Ok,
    Created,
    BadRequest,
    Forbidden,
    NotFound,
    InternalServerError,
}

impl StatusCode {
    fn reason(&self) -> &str {
        match self {
            StatusCode::Ok => "OK",
            StatusCode::Created => "Created",
            StatusCode::BadRequest => "Bad Request",
            StatusCode::Forbidden => "Forbidden",
            StatusCode::NotFound => "Not Found",
            StatusCode::InternalServerError => "Internal Server Error",
        }
    }

    fn code(&self) -> u16 {
        match self {
            StatusCode::Ok => 200,
            StatusCode::Created => 201,
            StatusCode::BadRequest => 400,
            StatusCode::Forbidden => 403,
            StatusCode::NotFound => 404,
            StatusCode::InternalServerError => 500,
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

    // RFC 9110 §15.3.2: 201 Created, a new resource has been created.
    #[test]
    fn created_code_and_reason() {
        assert_eq!(StatusCode::Created.code(), 201);
        assert_eq!(StatusCode::Created.reason(), "Created");
    }

    #[test]
    fn displays_created() {
        assert_eq!(StatusCode::Created.to_string(), "201 Created");
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
    fn permission_denied_code_and_reason() {
        assert_eq!(StatusCode::Forbidden.code(), 403);
        assert_eq!(StatusCode::Forbidden.reason(), "Forbidden");
    }
    #[test]
    fn internal_server_error_code_and_reason() {
        assert_eq!(StatusCode::InternalServerError.code(), 500);
        assert_eq!(
            StatusCode::InternalServerError.reason(),
            "Internal Server Error"
        );
    }

    #[test]
    fn displays_code_then_reason() {
        assert_eq!(StatusCode::Ok.to_string(), "200 OK");
        assert_eq!(StatusCode::Created.to_string(), "201 Created");
        assert_eq!(StatusCode::BadRequest.to_string(), "400 Bad Request");
        assert_eq!(StatusCode::Forbidden.to_string(), "403 Forbidden");
        assert_eq!(StatusCode::NotFound.to_string(), "404 Not Found");
        assert_eq!(
            StatusCode::InternalServerError.to_string(),
            "500 Internal Server Error"
        );
    }
}
