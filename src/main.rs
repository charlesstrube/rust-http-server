mod handlers;
mod http;

mod server;

use std::{net::TcpListener, thread};

use crate::server::handle_connection;

fn main() {
    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || handle_connection(stream));
            }
            Err(e) => println!("error: {}", e),
        }
    }
}
