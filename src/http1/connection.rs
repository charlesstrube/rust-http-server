use std::str::FromStr;

use crate::http::request::Request;

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

pub fn get_connection(request: &Request) -> Option<Vec<Connection>> {
    match request.header("Connection") {
        Some(headers) => {
            let list: Vec<Connection> = headers
                .split(',')
                .map(str::trim)
                .filter_map(|coding| coding.parse().ok())
                .collect();
            if !list.is_empty() { Some(list) } else { None }
        }
        None => None,
    }
}

pub fn wants_close(request: &Request) -> bool {
    match get_connection(request) {
        Some(headers) => headers.contains(&Connection::Close),
        _ => false,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::headers::Headers;

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

    fn with_header(name: &str, value: &str) -> Request {
        Request::new(
            "/",
            "GET",
            Headers::from(vec![(name.into(), value.into())]),
            "",
        )
        .unwrap()
    }

    fn bare() -> Request {
        Request::new("/", "GET", Headers::default(), "").unwrap()
    }

    // --- Connection (RFC 9110 §7.6.1) ---

    #[test]
    fn connection_absent_is_empty() {
        let request = bare();
        assert!(get_connection(&request).is_none());
        assert!(!wants_close(&request));
    }

    #[test]
    fn connection_close() {
        let request = with_header("Connection", "close");
        assert!(matches!(
            get_connection(&request).unwrap().as_slice(),
            [Connection::Close]
        ));
        assert!(wants_close(&request));
    }

    // RFC 9110 §5.1 and §7.6.1: field name and options are case-insensitive.
    #[test]
    fn connection_is_case_insensitive() {
        assert!(wants_close(&with_header("connection", "Close")));
    }

    #[test]
    fn connection_keep_alive_does_not_close() {
        let request = with_header("Connection", "keep-alive");
        assert!(matches!(
            get_connection(&request).unwrap().as_slice(),
            [Connection::KeepAlive]
        ));
        assert!(!wants_close(&request));
    }

    // RFC 9110 §7.6.1: `Connection` is a list; `close` counts wherever it appears.
    #[test]
    fn connection_close_anywhere_in_list() {
        assert!(wants_close(&with_header("Connection", "keep-alive, close")));
        assert!(wants_close(&with_header("Connection", "close, keep-alive")));
    }

    // The list keeps the order the client sent.
    #[test]
    fn connection_keeps_order() {
        let request = with_header("Connection", "keep-alive, close");
        assert!(matches!(
            get_connection(&request).unwrap().as_slice(),
            [Connection::KeepAlive, Connection::Close]
        ));
    }

    #[test]
    fn connection_list_tolerates_spaces() {
        assert!(wants_close(&with_header("Connection", "upgrade ,  close")));
    }

    #[test]
    fn connection_skips_unknown_options() {
        let request = with_header("Connection", "Upgrade, keep-alive");
        assert!(matches!(
            get_connection(&request).unwrap().as_slice(),
            [Connection::KeepAlive]
        ));
    }

    #[test]
    fn connection_with_only_unknown_options_is_empty() {
        let request = with_header("Connection", "upgrade");
        assert!(get_connection(&request).is_none());
        assert!(!wants_close(&request));
    }
}
