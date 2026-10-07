use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum StatusCode {
    Ok,
}

impl StatusCode {
    fn reason(&self) -> &str {
        match self {
            StatusCode::Ok => "OK",
        }
    }

    fn code(&self) -> u16 {
        match self {
            StatusCode::Ok => 200,
        }
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.code(), self.reason())
    }
}
