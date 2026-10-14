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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_get() {
        assert!(matches!(Method::try_from("GET"), Ok(Method::Get)));
    }

    #[test]
    fn parses_post() {
        assert!(matches!(Method::try_from("POST"), Ok(Method::Post)));
    }

    #[test]
    fn parses_from_owned_string() {
        assert!(matches!(Method::try_from(String::from("GET")), Ok(Method::Get)));
    }

    #[test]
    fn rejects_unknown_method() {
        assert!(matches!(
            Method::try_from("FOO"),
            Err(ParseMethodError::UnknownValue(_))
        ));
    }

    #[test]
    fn rejects_empty_method() {
        assert!(Method::try_from("").is_err());
    }

    #[test]
    fn displays_uppercase() {
        assert_eq!(Method::Get.to_string(), "GET");
        assert_eq!(Method::Post.to_string(), "POST");
    }
}
