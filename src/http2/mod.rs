pub mod frame;

use std::io::{BufRead, Write};

use crate::http::{request::Request, response::Response};

pub(crate) const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub fn serve(
    mut reader: impl BufRead, // reader
    mut writer: impl Write,   // writer
    handler: impl Fn(&Request) -> Response,
) {
    // read_frame(&mut reader, 0);
}
