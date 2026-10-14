use crate::http::{request::Request, response::Response, status::StatusCode};

pub fn route(request: &Request) -> Response {
    let status = match request.path() {
        "/" => StatusCode::Ok,
        _ => StatusCode::NotFound,
    };

    Response::new(status, vec![], String::new())
}
