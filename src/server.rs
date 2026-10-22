use std::str::FromStr;

use std::{
    io::{BufRead, BufReader},
    net::TcpStream,
};

use crate::config::Config;
use crate::handlers::route;
use crate::http::response::Response;
use std::io::Write;

use crate::http::request::{Request, RequestError};

fn read_request(reader: &mut impl BufRead) -> Result<Request, RequestError> {
    let mut head = String::new();
    loop {
        let read = reader
            .read_line(&mut head)
            .map_err(|err| RequestError::Io(err.to_string()))?;

        // ends if connection is lost of ends after the heqders
        if read == 0 || head.ends_with("\r\n\r\n") {
            break;
        }
    }

    let mut request = Request::from_str(&head)?;

    // Body bytes may already sit in the reader's buffer: read them from the reader, never the raw stream.
    let mut body = vec![0u8; request.content_length()?];

    reader
        .read_exact(&mut body)
        .map_err(|err| RequestError::Io(err.to_string()))?;

    let body = String::from_utf8(body).map_err(|_| RequestError::InvalidBody)?;

    request.set_body(body);

    Ok(request)
}

pub fn handle_connection(mut stream: TcpStream, config: &Config) {
    let mut buffer = BufReader::new(&stream);

    let response: Response = match read_request(&mut buffer) {
        Ok(request) => {
            let mut response = route(&request, config);

            response.set_encoding(request.accept_encoding());

            response
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.into()
        }
    };

    let result = stream.write_all(&response.into_bytes());

    if let Err(e) = result {
        eprintln!("error: {}", e);
    }
}
