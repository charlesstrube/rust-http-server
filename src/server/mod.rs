use std::{io::BufReader, net::TcpStream};

use crate::config::Config;
use crate::handlers::handle;
use crate::http::headers::encoding::with_gzip;
use crate::http1::serve;

pub fn handle_connection(stream: TcpStream, config: &Config) {
    let buffer = BufReader::new(&stream);
    let handler = with_gzip(move |request| handle(request, config));
    serve(buffer, &stream, handler);
}
