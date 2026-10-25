use std::str::FromStr;

#[derive(Clone, PartialEq)]
pub enum Connection {
    Close,
    KeepAlive,
}

pub enum ConnectionError {
    CannotParse,
}

impl FromStr for Connection {
    type Err = ConnectionError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "close" => Ok(Self::Close),
            "keep-alive" => Ok(Self::KeepAlive),
            _ => Err(ConnectionError::CannotParse),
        }
    }
}

impl From<Connection> for &str {
    fn from(value: Connection) -> Self {
        match value {
            Connection::Close => "close",
            Connection::KeepAlive => "keep-alive",
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_close() {
        assert!(matches!(
            "close".parse::<Connection>(),
            Ok(Connection::Close)
        ));
    }

    #[test]
    fn parses_keep_alive() {
        assert!(matches!(
            "keep-alive".parse::<Connection>(),
            Ok(Connection::KeepAlive)
        ));
    }

    // RFC 9110 §7.6.1: connection options are case-insensitive.
    #[test]
    fn parsing_is_case_insensitive() {
        assert!(matches!(
            "Close".parse::<Connection>(),
            Ok(Connection::Close)
        ));
        assert!(matches!(
            "CLOSE".parse::<Connection>(),
            Ok(Connection::Close)
        ));
        assert!(matches!(
            "Keep-Alive".parse::<Connection>(),
            Ok(Connection::KeepAlive)
        ));
    }

    // Other options exist (e.g. `Upgrade`): the server does not handle them.
    #[test]
    fn rejects_unknown_option() {
        assert!(matches!(
            "upgrade".parse::<Connection>(),
            Err(ConnectionError::CannotParse)
        ));
    }

    #[test]
    fn rejects_empty_value() {
        assert!(matches!(
            "".parse::<Connection>(),
            Err(ConnectionError::CannotParse)
        ));
    }

    // A list is not a single option: splitting it is the caller's job.
    #[test]
    fn rejects_list() {
        assert!(matches!(
            "keep-alive, close".parse::<Connection>(),
            Err(ConnectionError::CannotParse)
        ));
    }

    // Response headers use the canonical lowercase token, like `Encoding`.
    #[test]
    fn converts_to_canonical_token() {
        let close: &str = Connection::Close.into();
        let keep_alive: &str = Connection::KeepAlive.into();

        assert_eq!(close, "close");
        assert_eq!(keep_alive, "keep-alive");
    }

    #[test]
    fn parse_then_convert_round_trips() {
        let token: &str = "CLOSE".parse::<Connection>().ok().unwrap().into();
        assert_eq!(token, "close");
    }
}
