use crate::http::{response::Response, status::StatusCode};

#[derive(Debug, thiserror::Error)]
pub enum ParseEchoError {
    #[error("No content to print")]
    NoContent,
}

pub struct Echo {
    body: String,
    header: Vec<(String, String)>,
}

impl From<Echo> for Response {
    fn from(echo: Echo) -> Self {
        Response::new(StatusCode::Ok, echo.header, echo.body)
    }
}

impl TryFrom<String> for Echo {
    type Error = ParseEchoError;
    fn try_from(path: String) -> Result<Self, Self::Error> {
        let parsed = path.strip_prefix("/echo/");

        match parsed {
            Some(content) => Ok(Self {
                body: content.into(),
                header: vec![
                    ("Content-Type".into(), "text/plain".into()),
                    ("Content-Length".into(), content.len().to_string()),
                ],
            }),
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
