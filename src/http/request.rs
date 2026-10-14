use std::str::FromStr;

use crate::http::method::Method;

pub struct Request {
    path: String,
    method: Method,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum RequestError {
    #[error("Invalid method")]
    InvalidMethod,
    #[error("Cannot Parse")]
    CannotParse,
}

impl Request {
    pub fn new(path: &str, method: &str) -> Result<Self, RequestError> {
        let Ok(parsed_method) = Method::try_from(method) else {
            return Err(RequestError::InvalidMethod);
        };

        Ok(Self {
            path: path.into(),
            method: parsed_method,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn method(&self) -> &Method {
        &self.method
    }
}

impl FromStr for Request {
    type Err = RequestError;
    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let mut parts = line.split_whitespace();

        let (Some(method), Some(path), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(RequestError::CannotParse);
        };

        Request::new(path, method)
    }
}
