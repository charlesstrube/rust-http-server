use std::fmt;

#[derive(PartialEq, Debug)]
pub enum Protocol {
    Http1,
    Http2,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protocol::Http1 => write!(f, "HTTP/1.1"),
            Protocol::Http2 => write!(f, "HTTP/2"),
        }
    }
}
