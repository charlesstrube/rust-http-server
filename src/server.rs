use std::str::FromStr;

use std::{
    io::{BufRead, BufReader},
    net::TcpStream,
};

use crate::handlers::route;
use crate::http::response::Response;
use std::io::Write;

use crate::http::request::Request;

pub fn handle_connection(mut stream: TcpStream) {
    let mut head = String::new();
    let mut buffer = BufReader::new(&stream);

    loop {
        match buffer.read_line(&mut head) {
            Ok(line) => {
                if head.ends_with("\r\n\r\n") || line == 0 {
                    break;
                }
            }
            Err(err) => {
                println!("error: {}", err);
                break;
            }
        };
    }

    println!("final: {}", &head.as_str());

    let response = match Request::from_str(head.as_str()) {
        Ok(request) => route(&request),
        Err(err) => {
            println!("error: {}", err);
            Response::bad_request()
        }
    };

    let result = stream.write_all(&response.into_bytes());

    if let Err(e) = result {
        println!("error: {}", e);
    }
}
