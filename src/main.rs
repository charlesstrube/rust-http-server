use std::io::Write;
#[allow(unused_imports)]
use std::net::TcpListener;

use crate::http::response::Response;
use crate::http::status::StatusCode;

mod http;

fn main() {
    // You can use print statements as follows for debugging, they'll be visible when running tests.
    println!("Logs from your program will appear here!");

    // TODO: Uncomment the code below to pass the first stage
    //
    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let response = Response::new(StatusCode::Ok, vec![], "".to_string());
                let result = stream.write_all(&response.to_bytes());
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
