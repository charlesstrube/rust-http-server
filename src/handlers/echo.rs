use crate::http::{headers::Headers, response::Response, status::StatusCode};

#[derive(Debug, thiserror::Error)]
pub enum ParseEchoError {
    #[error("No content to print")]
    NoContent,
}

pub struct Echo {
    body: String,
    headers: Headers,
}

impl From<Echo> for Response {
    fn from(echo: Echo) -> Self {
        Response::new(StatusCode::Ok, echo.headers, echo.body)
    }
}

impl TryFrom<String> for Echo {
    type Error = ParseEchoError;
    fn try_from(path: String) -> Result<Self, Self::Error> {
        let parsed = path.strip_prefix("/echo/");

        match parsed {
            Some(content) => {
                if content.is_empty() {
                    return Err(ParseEchoError::NoContent);
                }

                let mut headers = Headers::default();
                headers.add("Content-Type", "text/plain");
                Ok(Self {
                    body: content.into(),
                    headers,
                })
            }
            None => Err(ParseEchoError::NoContent),
        }
    }
}

impl TryFrom<&str> for Echo {
    type Error = ParseEchoError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_string())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn echo(path: &str) -> Echo {
        Echo::try_from(path).unwrap()
    }

    #[test]
    fn extracts_string_after_prefix() {
        assert_eq!(echo("/echo/abc").body, "abc");
    }

    #[test]
    fn keeps_slashes_in_string() {
        assert_eq!(echo("/echo/a/b/c").body, "a/b/c");
    }

    #[test]
    fn sets_content_type_text_plain() {
        assert_eq!(
            echo("/echo/abc").headers.get("Content-Type"),
            Some("text/plain")
        );
    }

    #[test]
    fn parses_from_owned_string() {
        assert_eq!(
            Echo::try_from(String::from("/echo/abc")).unwrap().body,
            "abc"
        );
    }

    #[test]
    fn rejects_path_without_echo_prefix() {
        assert!(matches!(
            Echo::try_from("/abc"),
            Err(ParseEchoError::NoContent)
        ));
    }

    #[test]
    fn rejects_echo_without_trailing_slash() {
        assert!(matches!(
            Echo::try_from("/echo"),
            Err(ParseEchoError::NoContent)
        ));
    }

    #[test]
    fn rejects_empty_string() {
        assert!(matches!(
            Echo::try_from("/echo/"),
            Err(ParseEchoError::NoContent)
        ));
    }

    #[test]
    fn converts_into_200_response() {
        let response: Response = echo("/echo/abc").into();

        assert_eq!(response.status(), &StatusCode::Ok);
        assert_eq!(response.body(), "abc");
    }

    #[test]
    fn response_has_content_type_and_length() {
        let response: Response = echo("/echo/hello").into();

        assert_eq!(response.header("Content-Type"), Some("text/plain"));
        assert_eq!(response.header("Content-Length"), Some("5"));
    }
}
