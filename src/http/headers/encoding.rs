use std::str::FromStr;

use crate::http::{request::Request, response::Response};

#[derive(Clone, PartialEq)]
pub enum Encoding {
    Gzip,
}

pub enum EncodingError {
    CannotParse,
}

impl FromStr for Encoding {
    type Err = EncodingError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "gzip" => Ok(Self::Gzip),
            _ => Err(EncodingError::CannotParse),
        }
    }
}

impl From<Encoding> for &str {
    fn from(value: Encoding) -> Self {
        match value {
            Encoding::Gzip => "gzip",
        }
    }
}

// middleware
pub fn with_gzip(handler: impl Fn(&Request) -> Response) -> impl Fn(&Request) -> Response {
    move |request| {
        let mut response = handler(request);

        if request.has_accept_encoding_gzip() {
            response.set_encoding(Encoding::Gzip);
        }

        response
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_gzip() {
        assert!(matches!("gzip".parse::<Encoding>(), Ok(Encoding::Gzip)));
    }

    // RFC 9110 §8.4.1: content-coding values are case-insensitive.
    #[test]
    fn parsing_is_case_insensitive() {
        assert!(matches!("GZip".parse::<Encoding>(), Ok(Encoding::Gzip)));
        assert!(matches!("GZIP".parse::<Encoding>(), Ok(Encoding::Gzip)));
    }

    #[test]
    fn rejects_unknown_encoding() {
        assert!(matches!(
            "invalid-encoding".parse::<Encoding>(),
            Err(EncodingError::CannotParse)
        ));
    }

    #[test]
    fn rejects_empty_value() {
        assert!(matches!(
            "".parse::<Encoding>(),
            Err(EncodingError::CannotParse)
        ));
    }

    // A list is not a single coding: splitting it is the caller's job.
    #[test]
    fn rejects_list() {
        assert!(matches!(
            "gzip, deflate".parse::<Encoding>(),
            Err(EncodingError::CannotParse)
        ));
    }

    // Response headers use the canonical lowercase token, whatever the client sent.
    #[test]
    fn converts_to_canonical_token() {
        let token: &str = Encoding::Gzip.into();
        assert_eq!(token, "gzip");
    }

    #[test]
    fn parse_then_convert_round_trips() {
        let token: &str = "GZip".parse::<Encoding>().ok().unwrap().into();
        assert_eq!(token, "gzip");
    }
}
