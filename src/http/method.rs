use std::fmt;

pub enum Method {
    Get,
    Post,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseMethodError {
    #[error("unknown method value: {0}")]
    UnknownValue(String),
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Method::Get => write!(f, "GET"),
            Method::Post => write!(f, "POST"),
        }
    }
}

impl TryFrom<String> for Method {
    type Error = ParseMethodError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "get" => Ok(Self::Get),
            "post" => Ok(Method::Post),
            value => Err(ParseMethodError::UnknownValue(value.into())),
        }
    }
}

impl TryFrom<&str> for Method {
    type Error = ParseMethodError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_string())
    }
}
