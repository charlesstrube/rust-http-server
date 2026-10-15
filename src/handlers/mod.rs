use crate::{
    handlers::echo::Echo,
    http::{headers::Headers, request::Request, response::Response, status::StatusCode},
};

pub mod echo;

pub fn route(request: &Request) -> Response {
    let path = request.path();

    let headers = Headers::default();

    let response: Response = match path {
        "/" => Response::new(StatusCode::Ok, headers, String::new()),
        _ if path.starts_with("/echo/") => match Echo::try_from(path) {
            Ok(echo) => echo.into(),
            Err(_) => Response::new(StatusCode::NotFound, headers, String::new()),
        },
        _ => Response::new(StatusCode::NotFound, headers, String::new()),
    };

    response
}

#[cfg(test)]
mod test {
    use super::*;

    fn route_line(line: &str) -> Response {
        let request: Request = line.parse().unwrap();
        route(&request)
    }

    #[test]
    fn root_returns_200() {
        assert_eq!(route_line("GET / HTTP/1.1").status(), &StatusCode::Ok);
    }

    #[test]
    fn unknown_path_returns_404() {
        assert_eq!(
            route_line("GET /abcdefg HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn nested_unknown_path_returns_404() {
        assert_eq!(
            route_line("GET /a/b/c HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn root_matches_exactly() {
        assert_eq!(
            route_line("GET // HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn echo_returns_200() {
        assert_eq!(
            route_line("GET /echo/abc HTTP/1.1").status(),
            &StatusCode::Ok
        );
    }

    #[test]
    fn echo_returns_string_as_body() {
        assert_eq!(route_line("GET /echo/abc HTTP/1.1").body(), "abc");
    }

    #[test]
    fn echo_sets_content_type_text_plain() {
        assert_eq!(
            route_line("GET /echo/abc HTTP/1.1").header("Content-Type"),
            Some("text/plain")
        );
    }

    #[test]
    fn echo_sets_content_length() {
        assert_eq!(
            route_line("GET /echo/hello-world HTTP/1.1").header("Content-Length"),
            Some("11")
        );
    }

    #[test]
    fn echo_matches_spec_bytes() {
        assert_eq!(
            String::from_utf8(route_line("GET /echo/abc HTTP/1.1").into_bytes()).unwrap(),
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\n\r\nabc"
        );
    }

    #[test]
    fn echo_without_trailing_slash_returns_404() {
        assert_eq!(
            route_line("GET /echo HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn echo_prefix_must_be_a_full_segment() {
        assert_eq!(
            route_line("GET /echoabc HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }
}
