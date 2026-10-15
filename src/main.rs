mod handlers;
mod http;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::str::FromStr;

use crate::handlers::route;
use crate::http::request::Request;

fn main() {
    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let mut request_line = String::new();
                let mut buffer = BufReader::new(&stream);

                if let Err(err) = buffer.read_line(&mut request_line) {
                    println!("error: {}", err);
                    continue;
                };

                let request = match Request::from_str(request_line.as_str()) {
                    Ok(request) => request,
                    Err(err) => {
                        println!("error: {}", err);
                        continue;
                    }
                };

                let response = route(&request);

                let result = stream.write_all(&response.into_bytes());

                if let Err(e) = result {
                    println!("error: {}", e);
                }
            }
            Err(e) => {
                println!("error: {}", e);
            }
        }
    }
}
