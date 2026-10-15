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
