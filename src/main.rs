mod handlers;
mod http;

mod config;
mod server;
#[cfg(test)]
mod test_support;

use std::{net::TcpListener, process::exit, sync::Arc, thread};

use crate::{config::Config, server::handle_connection};

fn main() {
    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();
    let args = std::env::args().skip(1);

    let args: Vec<String> = args.collect();

    let config = Config::from_args(args);

    let config = match config {
        Ok(config) => config,
        Err(err) => {
            println!("config error: {}", err);
            exit(1);
        }
    };

    let config_arc = Arc::new(config);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let config = Arc::clone(&config_arc); // copie le pointeur, pas la Config
                thread::spawn(move || handle_connection(stream, config));
            }
            Err(e) => println!("error: {}", e),
        }
    }
}
