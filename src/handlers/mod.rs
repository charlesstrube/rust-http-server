use crate::{
    config::Config,
    handlers::{
        echo::Echo,
        files::{CreatedFile, StaticFile},
        user_agent::UserAgent,
    },
    http::{
        headers::Headers, method::Method, request::Request, response::Response, status::StatusCode,
    },
};

pub mod echo;
pub mod files;
pub mod user_agent;

fn respond<T, E>(result: Result<T, E>) -> Response
where
    T: Into<Response>,
    E: Into<Response>,
{
    match result {
        Ok(value) => value.into(),
        Err(error) => error.into(),
    }
}

fn route(request: &Request, config: &Config) -> Response {
    let path = request.path();
    let method = request.method();

    let Some(trimmed) = path.strip_prefix('/') else {
        return Response::bad_request();
    };

    let (segment, rest) = match trimmed.split_once("/") {
        Some((segment, rest)) => (segment, Some(rest)),
        None => (trimmed, None),
    };

    match (segment, method, rest) {
        ("", Method::Get, None) => Response::new(StatusCode::Ok, Headers::default(), String::new()),
        ("user-agent", Method::Get, None) => respond(UserAgent::try_from(request)),
        ("echo", Method::Get, Some(rest)) => respond(Echo::try_from(rest)),
        ("files", method, Some(rest)) => match config.directory() {
            Some(directory) => match method {
                Method::Get => respond(StaticFile::open(directory, rest)),
                Method::Post => respond(CreatedFile::create(directory, rest, request.body())),
                _ => Response::not_found(),
            },
            None => Response::not_found(),
        },
        _ => Response::not_found(),
    }
}

// Single entry point for every protocol: routing, then content negotiation (RFC 9110 §12.5.3).
pub fn handle(request: &Request, config: &Config) -> Response {
    let response = route(request, config);

    response
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::Config;
    use crate::http1::serialize::serialize_response;
    use crate::test_support::TempDir;
    // Test-only: raw HTTP/1.1 text is the most readable way to build a request.
    use crate::http1::parse::parse_request;

    fn route_line_with(config: &Config, line: &str) -> Response {
        let request = match parse_request(line) {
            Ok(request) => request,
            Err(e) => {
                eprintln!("err: {}", e);
                panic!();
            }
        };
        route(&request, config)
    }

    fn route_line(line: &str) -> Response {
        route_line_with(&Config::default(), line)
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
    fn path_without_leading_slash_returns_400() {
        assert_eq!(
            route_line("GET abc HTTP/1.1").status(),
            &StatusCode::BadRequest
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
        assert_eq!(route_line("GET /echo/abc HTTP/1.1").body_as_str(), "abc");
    }

    #[test]
    fn echo_sets_content_type_text_plain() {
        assert_eq!(
            route_line("GET /echo/abc HTTP/1.1").header("Content-Type"),
            Some("text/plain")
        );
    }

    #[test]
    fn echo_matches_spec_bytes() {
        assert_eq!(
            String::from_utf8(serialize_response(
                route_line("GET /echo/abc HTTP/1.1"),
                false
            ))
            .unwrap(),
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
    fn echo_with_empty_string_returns_404() {
        let response = route_line("GET /echo/ HTTP/1.1");

        assert_eq!(response.status(), &StatusCode::NotFound);
        assert_eq!(response.body_as_str(), "");
        assert_eq!(response.header("Content-Type"), None);
    }

    #[test]
    fn echo_prefix_must_be_a_full_segment() {
        assert_eq!(
            route_line("GET /echoabc HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn echo_ignores_request_headers() {
        assert_eq!(
            route_line("GET /echo/abc HTTP/1.1\r\nUser-Agent: foobar/1.2.3\r\n\r\n").body_as_str(),
            "abc"
        );
    }

    const UA_REQUEST: &str = "GET /user-agent HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: foobar/1.2.3\r\nAccept: */*\r\n\r\n";

    #[test]
    fn user_agent_returns_200() {
        assert_eq!(route_line(UA_REQUEST).status(), &StatusCode::Ok);
    }

    #[test]
    fn user_agent_returns_header_value_as_body() {
        assert_eq!(route_line(UA_REQUEST).body_as_str(), "foobar/1.2.3");
    }

    #[test]
    fn user_agent_matches_spec_bytes() {
        assert_eq!(
            String::from_utf8(serialize_response(route_line(UA_REQUEST), false)).unwrap(),
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 12\r\n\r\nfoobar/1.2.3"
        );
    }

    #[test]
    fn user_agent_without_header_returns_400() {
        let response = route_line("GET /user-agent HTTP/1.1\r\nHost: localhost:4221\r\n\r\n");

        assert_eq!(response.status(), &StatusCode::BadRequest);
        assert_eq!(response.body_as_str(), "");
        assert_eq!(response.header("Content-Type"), None);
    }

    #[test]
    fn user_agent_matches_path_exactly() {
        assert_eq!(
            route_line("GET /user-agent/x HTTP/1.1\r\nUser-Agent: a/1\r\n\r\n").status(),
            &StatusCode::NotFound
        );
    }

    // --- Files ---

    fn config_for(dir: &TempDir) -> Config {
        Config::from_args(vec!["--directory".to_string(), dir.path_str()]).unwrap()
    }

    fn dir_with_foo() -> TempDir {
        let dir = TempDir::new();
        dir.write("foo", "Hello, World!");
        dir
    }

    #[test]
    fn files_matches_spec_bytes() {
        let dir = dir_with_foo();

        assert_eq!(
            String::from_utf8(serialize_response(
                route_line_with(&config_for(&dir), "GET /files/foo HTTP/1.1"),
                false
            ))
            .unwrap(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 13\r\n\r\nHello, World!"
        );
    }

    #[test]
    fn files_missing_returns_404() {
        let dir = dir_with_foo();

        assert_eq!(
            route_line_with(&config_for(&dir), "GET /files/non_existant_file HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn files_without_configured_directory_returns_404() {
        assert_eq!(
            route_line("GET /files/foo HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn files_without_trailing_slash_returns_404() {
        let dir = dir_with_foo();

        assert_eq!(
            route_line_with(&config_for(&dir), "GET /files HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn files_prefix_must_be_a_full_segment() {
        let dir = dir_with_foo();
        dir.write("filesfoo", "nope");

        assert_eq!(
            route_line_with(&config_for(&dir), "GET /filesfoo HTTP/1.1").status(),
            &StatusCode::NotFound
        );
    }

    #[test]
    fn other_routes_still_work_with_directory() {
        let dir = dir_with_foo();

        assert_eq!(
            route_line_with(&config_for(&dir), "GET /echo/abc HTTP/1.1").body_as_str(),
            "abc"
        );
    }

    // --- Files: POST ---

    const SPEC_POST: &str = "POST /files/number HTTP/1.1\r\nHost: localhost:4221\r\nUser-Agent: curl/7.64.1\r\nAccept: */*\r\nContent-Type: application/octet-stream\r\nContent-Length: 5\r\n\r\n12345";

    fn post_files(name: &str, body: &str) -> String {
        format!(
            "POST /files/{name} HTTP/1.1\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn files_post_matches_spec_bytes() {
        let dir = TempDir::new();

        assert_eq!(
            String::from_utf8(serialize_response(
                route_line_with(&config_for(&dir), SPEC_POST),
                false
            ))
            .unwrap(),
            "HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn files_post_writes_body_to_file() {
        let dir = TempDir::new();

        route_line_with(&config_for(&dir), SPEC_POST);

        assert_eq!(dir.read("number"), "12345");
    }

    #[test]
    fn files_post_then_get_returns_contents() {
        let dir = TempDir::new();
        let config = config_for(&dir);

        route_line_with(&config, &post_files("file_123", "mango banana"));

        assert_eq!(
            route_line_with(&config, "GET /files/file_123 HTTP/1.1").body_as_str(),
            "mango banana"
        );
    }

    #[test]
    fn files_post_without_configured_directory_returns_404() {
        assert_eq!(route_line(SPEC_POST).status(), &StatusCode::NotFound);
    }

    #[test]
    fn files_post_traversal_returns_404() {
        let dir = TempDir::new();
        let public = dir.mkdir("public");
        let config = Config::from_args(vec![
            "--directory".to_string(),
            public.to_str().unwrap().to_string(),
        ])
        .unwrap();

        assert_eq!(
            route_line_with(&config, &post_files("../evil", "pwned")).status(),
            &StatusCode::NotFound
        );
        assert!(!dir.path().join("evil").exists());
    }

    // The method selects the handler: GET must never create anything.
    #[test]
    fn files_get_does_not_create_file() {
        let dir = TempDir::new();

        assert_eq!(
            route_line_with(&config_for(&dir), "GET /files/ghost HTTP/1.1").status(),
            &StatusCode::NotFound
        );
        assert!(!dir.path().join("ghost").exists());
    }

    // The method selects the handler: POST must never answer with the old contents.
    #[test]
    fn files_post_on_existing_file_does_not_return_contents() {
        let dir = dir_with_foo();

        let response = route_line_with(&config_for(&dir), &post_files("foo", "new"));

        assert_eq!(response.status(), &StatusCode::Created);
        assert_eq!(response.body_as_str(), "");
    }
}
