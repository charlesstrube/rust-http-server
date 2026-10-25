pub mod protocol;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::net::TcpStream;

use crate::config::Config;
use crate::handlers::handle;
use crate::http::headers::encoding::with_gzip;
use crate::http1::serve;
use crate::http2::PREFACE;
use crate::server::protocol::Protocol;

fn detect(reader: &mut impl BufRead) -> io::Result<Option<(Protocol, Vec<u8>)>> {
    let mut index = 0;
    let mut http_protocol: Option<Protocol> = None;
    let mut buffer = vec![];

    loop {
        let mut byte = [0u8; 1];
        match reader.read(&mut byte)? {
            0 => {
                if !buffer.is_empty() {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                break;
            }
            _ => {
                println!("byte: {}, index: {}", byte[0], index);
                buffer.push(byte[0]);
                if PREFACE[index] == byte[0] && http_protocol.is_none() {
                    println!("protocol: http2");
                    http_protocol = Some(Protocol::Http2);
                } else if PREFACE[index] != byte[0] {
                    println!("protocol: http1");
                    http_protocol = Some(Protocol::Http1);
                    break;
                }
                if index == 23 {
                    println!("break 24");
                    break;
                }

                index += 1;
            }
        }
    }

    match http_protocol {
        Some(protocol) => {
            println!("protocol final: {}", protocol);

            Ok(Some((protocol, buffer)))
        }
        _ => {
            println!("final none");
            Ok(None)
        }
    }
}

pub fn handle_connection(stream: TcpStream, config: &Config) {
    let mut buffer = BufReader::new(&stream);
    match detect(&mut buffer) {
        Ok(Some((Protocol::Http1, consumed))) => {
            let reader = Cursor::new(consumed).chain(&mut buffer);

            let handler = with_gzip(move |request| handle(request, config));
            serve(reader, &stream, handler);
        }
        Ok(Some((Protocol::Http2, _))) => {
            println!("cool, http2");
            unimplemented!()
        }
        Ok(None) => {
            println!("cool, EOF");
            unimplemented!()
        }
        Err(_) => {
            println!("cool, error");
            unimplemented!()
        }
    };
}

#[cfg(test)]
mod test {

    use super::*;
    use std::io::{BufReader, Cursor, Read};

    // RFC 9113 §3.4
    const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

    // Rebuilds the original input from what `detect` consumed and what is left.
    fn rebuild(consumed: Vec<u8>, rest: impl Read) -> Vec<u8> {
        let mut bytes = Vec::new();
        Cursor::new(consumed)
            .chain(rest)
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }

    #[test]
    fn full_preface_is_http2() {
        let mut reader = Cursor::new(PREFACE);

        let (protocol, consumed) = detect(&mut reader).unwrap().unwrap();

        assert_eq!(protocol, Protocol::Http2);
        assert_eq!(consumed, PREFACE);
    }

    // What follows the preface are frames: they belong to the HTTP/2 codec.
    #[test]
    fn http2_does_not_consume_past_the_preface() {
        let settings_frame = [0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00];
        let input = [PREFACE, &settings_frame].concat();
        let mut reader = Cursor::new(input.as_slice());

        let (protocol, consumed) = detect(&mut reader).unwrap().unwrap();

        assert_eq!(protocol, Protocol::Http2);
        assert_eq!(consumed, PREFACE);
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, settings_frame);
    }

    // Worst-case TCP fragmentation: every read returns a single byte.
    #[test]
    fn preface_received_byte_by_byte_is_http2() {
        let mut reader = BufReader::with_capacity(1, Cursor::new(PREFACE));

        let (protocol, _) = detect(&mut reader).unwrap().unwrap();

        assert_eq!(protocol, Protocol::Http2);
    }

    #[test]
    fn get_request_is_http1() {
        let mut reader = Cursor::new(b"GET / HTTP/1.1\r\n\r\n".as_slice());

        let (protocol, _) = detect(&mut reader).unwrap().unwrap();

        assert_eq!(protocol, Protocol::Http1);
    }

    // These methods share the leading 'P' with the preface.
    #[test]
    fn methods_starting_with_p_are_http1() {
        for raw in [
            "POST /files/a HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc",
            "PUT / HTTP/1.1\r\n\r\n",
            "PATCH / HTTP/1.1\r\n\r\n",
        ] {
            let mut reader = Cursor::new(raw.as_bytes());

            let (protocol, _) = detect(&mut reader).unwrap().unwrap();

            assert_eq!(protocol, Protocol::Http1, "for {raw:?}");
        }
    }

    // Matches the first 4 bytes of the preface, then diverges.
    #[test]
    fn pri_method_without_preface_is_http1() {
        let mut reader = Cursor::new(b"PRI / HTTP/1.1\r\n\r\n".as_slice());

        let (protocol, _) = detect(&mut reader).unwrap().unwrap();

        assert_eq!(protocol, Protocol::Http1);
    }

    // The input ends right after the first mismatching byte: deciding must not
    // need more data, otherwise a short HTTP/1.1 request would block forever.
    #[test]
    fn decides_on_the_first_mismatching_byte() {
        for raw in ["G", "PO", "PRI /"] {
            let mut reader = Cursor::new(raw.as_bytes());

            let (protocol, consumed) = detect(&mut reader).unwrap().unwrap();

            assert_eq!(protocol, Protocol::Http1, "for {raw:?}");
            assert_eq!(consumed, raw.as_bytes(), "for {raw:?}");
        }
    }

    // Nothing read by `detect` may be lost for the HTTP/1.1 codec.
    #[test]
    fn http1_input_is_rebuilt_intact() {
        let raw = b"POST /files/a HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET / HTTP/1.1\r\n\r\n";
        let mut reader = Cursor::new(raw.as_slice());

        let (_, consumed) = detect(&mut reader).unwrap().unwrap();

        assert_eq!(rebuild(consumed, reader), raw);
    }

    #[test]
    fn http1_request_is_parsed_from_the_chained_reader() {
        let mut reader =
            Cursor::new(b"PUT /files/a HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi".as_slice());

        let (_, consumed) = detect(&mut reader).unwrap().unwrap();
        let mut chained = Cursor::new(consumed).chain(reader);
        let request = crate::http1::read_request(&mut chained).unwrap().unwrap();

        assert_eq!(request.path(), "/files/a");
        assert_eq!(request.body(), "hi");
    }

    // A client that connects and closes without sending anything.
    #[test]
    fn empty_input_is_none() {
        let mut reader = Cursor::new(b"".as_slice());

        assert!(matches!(detect(&mut reader), Ok(None)));
    }

    #[test]
    fn eof_in_the_middle_of_the_preface_is_an_error() {
        let mut reader = Cursor::new(&PREFACE[..12]);

        let error = detect(&mut reader).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
