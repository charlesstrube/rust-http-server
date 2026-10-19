mod handlers;
mod http;

mod config;
mod server;
#[cfg(test)]
mod test_support;

use std::{net::TcpListener, process::exit, sync::Arc, thread};

use crate::{config::Config, server::handle_connection};

fn main() {
    let args = std::env::args().skip(1);

    let config = Config::from_args(args);

    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();
    let config = match config {
        Ok(config) => config,
        Err(err) => {
            eprintln!("config error: {}", err);
            exit(1);
        }
    };

    let config_arc = Arc::new(config);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // copu the pointer, not the config itself
                let config = Arc::clone(&config_arc);

                thread::spawn(move || handle_connection(stream, &config));
            }
            Err(e) => eprintln!("error: {}", e),
        }
    }
}
