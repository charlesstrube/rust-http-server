use crate::http::{
    headers::{Headers, ParseHeadersError},
    request::{Request, RequestError},
};

pub fn parse_request(head: &str) -> Result<Request, RequestError> {
    let (first_line, headers, body) = match head.split_once("\r\n\r\n") {
        Some((rest, body)) => match rest.split_once("\r\n") {
            Some((first_line, headers)) => {
                (first_line, parse_headers(headers)?, String::from(body))
            }
            None => (head, Headers::default(), String::new()),
        },
        None => match head.split_once("\r\n") {
            Some((first_line, header_lines)) => {
                (first_line, parse_headers(header_lines)?, String::new())
            }
            None => (head, Headers::default(), String::new()),
        },
    };

    let mut parts = first_line.split_whitespace();

    let (Some(method), Some(path), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(RequestError::CannotParse);
    };

    Request::new(path, method, headers, body.as_str())
}

pub fn parse_headers(block: &str) -> Result<Headers, ParseHeadersError> {
    let headers = block
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

    Ok(Headers::from(headers))
}

pub fn content_length(request: &Request) -> Result<usize, RequestError> {
    match request.header("Content-Length") {
        Some(value) => match value.parse::<usize>() {
            Ok(int) => Ok(int),
            Err(_) => Err(RequestError::InvalidContentLength),
        },
        None => Ok(0),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::method::Method;

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

    // --- Request head (RFC 9112 §2.1, §3) ---

    #[test]
    fn parses_root_path() {
        let request = parse_request("GET / HTTP/1.1").unwrap();
        assert_eq!(request.path(), "/");
        assert!(matches!(request.method(), Method::Get));
    }

    #[test]
    fn parses_custom_path() {
        let request = parse_request("GET /abcdefg HTTP/1.1").unwrap();
        assert_eq!(request.path(), "/abcdefg");
    }

    #[test]
    fn parses_post() {
        let request = parse_request("POST /files/a HTTP/1.1").unwrap();
        assert!(matches!(request.method(), Method::Post));
    }

    // read_line keeps the line terminator.
    #[test]
    fn ignores_trailing_crlf() {
        let request = parse_request("GET /index.html HTTP/1.1\r\n").unwrap();
        assert_eq!(request.path(), "/index.html");
    }

    #[test]
    fn rejects_unknown_method() {
        let result = parse_request("FOO / HTTP/1.1");
        assert_eq!(result.err(), Some(RequestError::InvalidMethod));
    }

    #[test]
    fn rejects_empty_line() {
        let result = parse_request("");
        assert_eq!(result.err(), Some(RequestError::CannotParse));
    }

    #[test]
    fn rejects_blank_line() {
        let result = parse_request("\r\n");
        assert_eq!(result.err(), Some(RequestError::CannotParse));
    }

    #[test]
    fn rejects_missing_version() {
        let result = parse_request("GET /");
        assert_eq!(result.err(), Some(RequestError::CannotParse));
    }

    // Parsing a full request head: request line, field lines, empty line (RFC 9112 §2.1).

    const SPEC_REQUEST: &str = "GET /user-agent HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: foobar/1.2.3\r\nAccept: */*\r\n\r\n";

    #[test]
    fn parses_request_line_of_full_head() {
        let request = parse_request(SPEC_REQUEST).unwrap();
        assert_eq!(request.path(), "/user-agent");
        assert!(matches!(request.method(), Method::Get));
    }

    #[test]
    fn parses_headers_after_request_line() {
        let request = parse_request(SPEC_REQUEST).unwrap();

        assert_eq!(request.header("Host"), Some("localhost:4221"));
        assert_eq!(request.header("User-Agent"), Some("foobar/1.2.3"));
        assert_eq!(request.header("Accept"), Some("*/*"));
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    #[test]
    fn header_lookup_is_case_insensitive() {
        let request = parse_request(SPEC_REQUEST).unwrap();
        assert_eq!(request.header("user-agent"), Some("foobar/1.2.3"));
        assert_eq!(request.header("USER-AGENT"), Some("foobar/1.2.3"));
    }

    #[test]
    fn missing_header_is_none() {
        let request = parse_request(SPEC_REQUEST).unwrap();
        assert_eq!(request.header("X-Unknown"), None);
    }

    #[test]
    fn request_line_only_has_no_headers() {
        let request = parse_request("GET / HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(request.header("User-Agent"), None);
    }

    #[test]
    fn rejects_invalid_header_line() {
        let result = parse_request("GET / HTTP/1.1\r\ngarbage\r\n\r\n");
        assert!(matches!(result, Err(RequestError::InvalidHeader(_))));
    }

    // --- Body (RFC 9112 §6): everything after the empty line closing the head ---

    const SPEC_POST: &str = "POST /files/number HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: curl/7.64.1\r\nAccept: */*\r\nContent-Type: application/octet-stream\r\nContent-Length: 5\r\n\r\n12345";

    #[test]
    fn parses_body_after_empty_line() {
        let request = parse_request(SPEC_POST).unwrap();
        assert_eq!(request.body(), "12345");
    }

    #[test]
    fn parses_request_line_and_headers_of_request_with_body() {
        let request = parse_request(SPEC_POST).unwrap();

        assert!(matches!(request.method(), Method::Post));
        assert_eq!(request.path(), "/files/number");
        assert_eq!(
            request.header("Content-Type"),
            Some("application/octet-stream")
        );
        assert_eq!(request.header("Content-Length"), Some("5"));
    }

    // The body is opaque data: it must never reach the header parser.
    #[test]
    fn body_is_not_parsed_as_headers() {
        let request =
            parse_request("POST /files/a HTTP/1.1\r\nContent-Length: 7\r\n\r\ngarbage").unwrap();
        assert_eq!(request.body(), "garbage");
    }

    // Only the first empty line ends the head; later CRLFs belong to the body.
    #[test]
    fn body_may_contain_crlf_and_empty_lines() {
        let request =
            parse_request("POST /files/a HTTP/1.1\r\nContent-Length: 10\r\n\r\na\r\nb\r\n\r\nc")
                .unwrap();
        assert_eq!(request.body(), "a\r\nb\r\n\r\nc");
    }

    #[test]
    fn head_without_body_has_empty_body() {
        let request = parse_request(SPEC_REQUEST).unwrap();
        assert_eq!(request.body(), "");
    }

    #[test]
    fn request_line_only_has_empty_body() {
        let request = parse_request("GET / HTTP/1.1").unwrap();
        assert_eq!(request.body(), "");
    }

    #[test]
    fn request_line_with_empty_line_has_empty_body() {
        let request = parse_request("GET / HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(request.body(), "");
    }

    // --- Header section (RFC 9112 §5) ---

    // Parsing a header section, RFC 9112 §5:
    // field-line = field-name ":" OWS field-value OWS

    fn parse(block: &str) -> Headers {
        parse_headers(block).unwrap()
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
            parse_headers("garbage\r\n"),
            Err(ParseHeadersError::MissingColon)
        ));
    }

    // RFC 9112 §5.1: no whitespace is allowed between the field name and the colon.
    #[test]
    fn rejects_whitespace_before_colon() {
        assert!(matches!(
            parse_headers("Host : localhost\r\n"),
            Err(ParseHeadersError::InvalidName)
        ));
    }

    #[test]
    fn rejects_empty_name() {
        assert!(matches!(
            parse_headers(": x\r\n"),
            Err(ParseHeadersError::InvalidName)
        ));
    }

    // A field name is a token (RFC 9110 §5.1): whitespace is never allowed inside it.
    #[test]
    fn rejects_whitespace_inside_name() {
        assert!(matches!(
            parse_headers("Ho st: localhost\r\n"),
            Err(ParseHeadersError::InvalidName)
        ));
    }

    // RFC 9112 §5.2: a line starting with whitespace is an obsolete line folding (obs-fold).
    #[test]
    fn rejects_leading_whitespace_before_name() {
        assert!(matches!(
            parse_headers("Host: localhost\r\n User-Agent: foobar/1.2.3\r\n"),
            Err(ParseHeadersError::InvalidName)
        ));
    }

    #[test]
    fn rejects_whole_block_if_one_line_is_invalid() {
        assert!(parse_headers("A: 1\r\ngarbage\r\nB: 2\r\n").is_err());
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

    // --- Content-Length (RFC 9110 §8.6, RFC 9112 §6.3) ---

    #[test]
    fn content_length_reads_header_value() {
        assert_eq!(content_length(&with_header("Content-Length", "5")), Ok(5));
    }

    // RFC 9112 §6.3 rule 7: a request without Content-Length (nor Transfer-Encoding) has no body.
    #[test]
    fn missing_content_length_means_zero() {
        assert_eq!(content_length(&bare()), Ok(0));
    }

    #[test]
    fn content_length_name_is_case_insensitive() {
        assert_eq!(content_length(&with_header("content-length", "3")), Ok(3));
    }

    // RFC 9110 §8.6: Content-Length = 1*DIGIT.
    #[test]
    fn rejects_non_numeric_content_length() {
        assert_eq!(
            content_length(&with_header("Content-Length", "abc")),
            Err(RequestError::InvalidContentLength)
        );
    }

    #[test]
    fn rejects_negative_content_length() {
        assert_eq!(
            content_length(&with_header("Content-Length", "-1")),
            Err(RequestError::InvalidContentLength)
        );
    }
}
