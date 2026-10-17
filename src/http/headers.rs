use std::{fmt::Display, str::FromStr};

#[derive(Default)]
pub struct Headers(Vec<(String, String)>);

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseHeadersError {
    #[error("Missing colon in header line")]
    MissingColon,
    #[error("Invalid header name")]
    InvalidName,
}

impl From<Vec<(String, String)>> for Headers {
    fn from(value: Vec<(String, String)>) -> Self {
        Self(value)
    }
}

impl FromStr for Headers {
    type Err = ParseHeadersError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let headers = s
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let value = line
                    .split_once(":")
                    .ok_or(ParseHeadersError::MissingColon)?;

                if value.0.trim().is_empty() {
                    return Err(ParseHeadersError::InvalidName);
                }

                if value.0.contains(char::is_whitespace) {
                    return Err(ParseHeadersError::InvalidName);
                }

                Ok((String::from(value.0.trim()), String::from(value.1.trim())))
            })
            .collect::<Result<Vec<(String, String)>, ParseHeadersError>>()?;

        Ok(Self(headers))
    }
}

impl Display for Headers {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (key, value) in &self.0 {
            write!(f, "{}: {}\r\n", key, value)?;
        }
        Ok(())
    }
}

impl Headers {
    pub fn add(&mut self, key: &str, value: &str) {
        self.0.push((key.into(), value.into()));
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find_map(|(k, v)| {
            if k.eq_ignore_ascii_case(key) {
                Some(v.as_str())
            } else {
                None
            }
        })
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let mut found = false;
        self.0.retain_mut(|(current_key, current_value)| {
            if current_key.eq_ignore_ascii_case(key) {
                if !found {
                    *current_value = value.into();
                    found = true;
                    return true;
                }

                false
            } else {
                true
            }
        });

        if !found {
            self.add(key, value);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // Case-insensitive count of field lines named `name` in the serialized headers.
    fn count_lines(headers: &Headers, name: &str) -> usize {
        let prefix = format!("{}:", name.to_ascii_lowercase());
        headers
            .to_string()
            .to_ascii_lowercase()
            .split("\r\n")
            .filter(|line| line.starts_with(&prefix))
            .count()
    }

    #[test]
    fn new_is_empty() {
        assert_eq!(Headers::from(vec![]).to_string(), "");
    }

    #[test]
    fn default_is_empty() {
        assert_eq!(Headers::default().to_string(), "");
    }

    #[test]
    fn get_returns_added_value() {
        let headers = Headers::from(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.get("Content-Type"), Some("text/plain"));
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    #[test]
    fn get_is_case_insensitive() {
        let headers = Headers::from(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.get("content-type"), Some("text/plain"));
        assert_eq!(headers.get("CONTENT-TYPE"), Some("text/plain"));
    }

    #[test]
    fn get_missing_is_none() {
        assert_eq!(Headers::default().get("Content-Type"), None);
    }

    #[test]
    fn displays_one_crlf_terminated_line_per_header() {
        let headers = Headers::from(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.to_string(), "Content-Type: text/plain\r\n");
    }

    #[test]
    fn displays_in_insertion_order() {
        let headers = Headers::from(vec![("B".into(), "2".into()), ("A".into(), "1".into())]);

        assert_eq!(headers.to_string(), "B: 2\r\nA: 1\r\n");
    }

    #[test]
    fn displays_name_with_original_case() {
        let headers = Headers::from(vec![("x-CuStOm".into(), "v".into())]);

        assert_eq!(headers.to_string(), "x-CuStOm: v\r\n");
    }

    // RFC 9110 §5.3: a field may appear several times (e.g. Set-Cookie).
    #[test]
    fn add_keeps_duplicates() {
        let mut headers = Headers::default();
        headers.add("Set-Cookie", "a=1");
        headers.add("Set-Cookie", "b=2");

        assert_eq!(
            headers.to_string(),
            "Set-Cookie: a=1\r\nSet-Cookie: b=2\r\n"
        );
    }

    #[test]
    fn get_returns_first_value_of_duplicates() {
        let headers = Headers::from(vec![
            ("Set-Cookie".into(), "a=1".into()),
            ("Set-Cookie".into(), "b=2".into()),
        ]);

        assert_eq!(headers.get("Set-Cookie"), Some("a=1"));
    }

    #[test]
    fn set_adds_missing_header() {
        let mut headers = Headers::default();
        headers.set("Content-Length", "3");

        assert_eq!(headers.get("Content-Length"), Some("3"));
    }

    #[test]
    fn set_replaces_existing_value() {
        let mut headers = Headers::default();
        headers.add("Content-Length", "999");
        headers.set("Content-Length", "3");

        assert_eq!(headers.get("Content-Length"), Some("3"));
        assert_eq!(count_lines(&headers, "Content-Length"), 1);
    }

    #[test]
    fn set_matches_existing_name_case_insensitively() {
        let mut headers = Headers::default();
        headers.add("content-length", "999");
        headers.set("Content-Length", "3");

        assert_eq!(headers.get("Content-Length"), Some("3"));
        assert_eq!(count_lines(&headers, "Content-Length"), 1);
    }

    #[test]
    fn set_collapses_duplicates_into_one() {
        let mut headers = Headers::default();
        headers.add("Content-Length", "1");
        headers.add("Content-Length", "2");
        headers.set("Content-Length", "3");

        assert_eq!(count_lines(&headers, "Content-Length"), 1);
        assert_eq!(headers.get("Content-Length"), Some("3"));
    }

    #[test]
    fn set_leaves_other_headers_untouched() {
        let mut headers = Headers::default();
        headers.add("Content-Type", "text/plain");
        headers.set("Content-Length", "3");

        assert_eq!(headers.get("Content-Type"), Some("text/plain"));
    }

    // Parsing a header section, RFC 9112 §5:
    // field-line = field-name ":" OWS field-value OWS

    fn parse(block: &str) -> Headers {
        block.parse().unwrap()
    }

    #[test]
    fn parses_empty_block() {
        assert_eq!(parse("").to_string(), "");
    }

    #[test]
    fn parses_single_field_line() {
        assert_eq!(
            parse("User-Agent: foobar/1.2.3\r\n").get("User-Agent"),
            Some("foobar/1.2.3")
        );
    }

    #[test]
    fn parses_multiple_field_lines_in_order() {
        let block = "Host: localhost:4221\r\nUser-Agent: foobar/1.2.3\r\nAccept: */*\r\n";

        assert_eq!(parse(block).to_string(), block);
    }

    #[test]
    fn parses_without_trailing_crlf() {
        assert_eq!(parse("A: 1\r\nB: 2").get("B"), Some("2"));
    }

    // The empty line closing the header section is not a field line.
    #[test]
    fn ignores_terminating_empty_line() {
        assert_eq!(parse("A: 1\r\n\r\n").to_string(), "A: 1\r\n");
    }

    // Only the first colon separates the name from the value.
    #[test]
    fn value_may_contain_colons() {
        assert_eq!(
            parse("Host: localhost:4221\r\n").get("Host"),
            Some("localhost:4221")
        );
    }

    #[test]
    fn trims_optional_whitespace_around_value() {
        assert_eq!(
            parse("User-Agent: \t foobar/1.2.3 \t\r\n").get("User-Agent"),
            Some("foobar/1.2.3")
        );
    }

    #[test]
    fn whitespace_after_colon_is_optional() {
        assert_eq!(
            parse("User-Agent:foobar/1.2.3\r\n").get("User-Agent"),
            Some("foobar/1.2.3")
        );
    }

    #[test]
    fn keeps_inner_whitespace_in_value() {
        assert_eq!(
            parse("User-Agent: Mozilla/5.0 (X11; Linux x86_64)\r\n").get("User-Agent"),
            Some("Mozilla/5.0 (X11; Linux x86_64)")
        );
    }

    #[test]
    fn accepts_empty_value() {
        assert_eq!(parse("X-Empty:\r\n").get("X-Empty"), Some(""));
    }

    #[test]
    fn parsed_names_keep_original_case() {
        assert_eq!(parse("user-agent: x\r\n").to_string(), "user-agent: x\r\n");
    }

    #[test]
    fn parsed_names_are_matched_case_insensitively() {
        assert_eq!(parse("user-agent: x\r\n").get("User-Agent"), Some("x"));
    }

    #[test]
    fn parses_duplicate_field_lines() {
        let headers = parse("Set-Cookie: a=1\r\nSet-Cookie: b=2\r\n");

        assert_eq!(count_lines(&headers, "Set-Cookie"), 2);
    }

    #[test]
    fn rejects_line_without_colon() {
        assert!(matches!(
            "garbage\r\n".parse::<Headers>(),
            Err(ParseHeadersError::MissingColon)
        ));
    }

    // RFC 9112 §5.1: no whitespace is allowed between the field name and the colon.
    #[test]
    fn rejects_whitespace_before_colon() {
        assert!(matches!(
            "Host : localhost\r\n".parse::<Headers>(),
            Err(ParseHeadersError::InvalidName)
        ));
    }

    #[test]
    fn rejects_empty_name() {
        assert!(matches!(
            ": x\r\n".parse::<Headers>(),
            Err(ParseHeadersError::InvalidName)
        ));
    }

    // A field name is a token (RFC 9110 §5.1): whitespace is never allowed inside it.
    #[test]
    fn rejects_whitespace_inside_name() {
        assert!(matches!(
            "Ho st: localhost\r\n".parse::<Headers>(),
            Err(ParseHeadersError::InvalidName)
        ));
    }

    // RFC 9112 §5.2: a line starting with whitespace is an obsolete line folding (obs-fold).
    #[test]
    fn rejects_leading_whitespace_before_name() {
        assert!(matches!(
            "Host: localhost\r\n User-Agent: foobar/1.2.3\r\n".parse::<Headers>(),
            Err(ParseHeadersError::InvalidName)
        ));
    }

    #[test]
    fn rejects_whole_block_if_one_line_is_invalid() {
        assert!("A: 1\r\ngarbage\r\nB: 2\r\n".parse::<Headers>().is_err());
    }
}
