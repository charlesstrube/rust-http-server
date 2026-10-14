use crate::http::{request::Request, response::Response, status::StatusCode};

pub fn route(request: &Request) -> Response {
    let status = match request.path() {
        "/" => StatusCode::Ok,
        _ => StatusCode::NotFound,
    };

    Response::new(status, vec![], String::new())
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
        assert_eq!(route_line("GET // HTTP/1.1").status(), &StatusCode::NotFound);
    }
}
