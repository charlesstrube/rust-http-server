use crate::http::{headers::Headers, request::Request, response::Response, status::StatusCode};

pub struct UserAgent {
    value: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseUserAgentError {
    #[error("Missing User-Agent header")]
    MissingHeader,
}

impl TryFrom<&Request> for UserAgent {
    type Error = ParseUserAgentError;

    fn try_from(request: &Request) -> Result<Self, Self::Error> {
        match request.header("User-Agent") {
            Some(ua) => Ok(Self { value: ua.into() }),
            None => Err(ParseUserAgentError::MissingHeader),
        }
    }
}

impl From<ParseUserAgentError> for Response {
    fn from(_error: ParseUserAgentError) -> Self {
        Response::new(StatusCode::BadRequest, Headers::default(), String::new())
    }
}

impl From<UserAgent> for Response {
    fn from(user_agent: UserAgent) -> Self {
        Self::new(
            StatusCode::Ok,
            Headers::from(vec![(
                String::from("Content-Type"),
                String::from("text/plain"),
            )]),
            user_agent.value,
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::{request::Request, response::Response, status::StatusCode};

    const SPEC_REQUEST: &str = "GET /user-agent HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: foobar/1.2.3\r\nAccept: */*\r\n\r\n";

    fn request(raw: &str) -> Request {
        raw.parse().unwrap()
    }

    fn response(raw: &str) -> Response {
        UserAgent::try_from(&request(raw)).unwrap().into()
    }

    #[test]
    fn converts_into_200_response() {
        assert_eq!(response(SPEC_REQUEST).status(), &StatusCode::Ok);
    }

    #[test]
    fn body_is_user_agent_value() {
        assert_eq!(response(SPEC_REQUEST).body(), "foobar/1.2.3");
    }

    #[test]
    fn sets_content_type_text_plain() {
        assert_eq!(
            response(SPEC_REQUEST).header("Content-Type"),
            Some("text/plain")
        );
    }

    #[test]
    fn sets_content_length_to_value_length() {
        assert_eq!(response(SPEC_REQUEST).header("Content-Length"), Some("12"));
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    #[test]
    fn header_name_is_case_insensitive() {
        let raw = "GET /user-agent HTTP/1.1\r\nuser-agent: foobar/1.2.3\r\n\r\n";

        assert_eq!(response(raw).body(), "foobar/1.2.3");
    }

    #[test]
    fn keeps_spaces_inside_value() {
        let raw = "GET /user-agent HTTP/1.1\r\nUser-Agent: Mozilla/5.0 (X11; Linux x86_64)\r\n\r\n";

        assert_eq!(response(raw).body(), "Mozilla/5.0 (X11; Linux x86_64)");
    }

    #[test]
    fn rejects_missing_header() {
        let request = request("GET /user-agent HTTP/1.1\r\nHost: localhost:4221\r\n\r\n");

        assert!(matches!(
            UserAgent::try_from(&request),
            Err(ParseUserAgentError::MissingHeader)
        ));
    }

    #[test]
    fn matches_spec_bytes() {
        assert_eq!(
            String::from_utf8(response(SPEC_REQUEST).into_bytes()).unwrap(),
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 12\r\n\r\nfoobar/1.2.3"
        );
    }
}
