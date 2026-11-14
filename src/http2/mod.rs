pub mod assembler;
pub mod connection;
pub mod error;
pub mod frame;
pub mod hpack;
pub mod serialize;
pub mod settings;

use std::io::{BufRead, Write};

use crate::{
    http::{request::Request, response::Response},
    http2::{
        assembler::Assembler,
        connection::{Decision, handle_frame},
        error::{ErrorCode, goaway, rst_stream},
        frame::{Flags, Frame, FrameType, read_frame},
        serialize::serialize_response,
        settings::Settings,
    },
};

pub(crate) const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

fn fail_connection(writer: &mut impl Write, last_stream_id: u32, code: ErrorCode) {
    let frame = goaway(last_stream_id, code, &[]);
    send_frames(writer, &[frame]);
}

fn send_frames(writer: &mut impl Write, frames: &[Frame]) -> bool {
    for frame in frames.iter() {
        if writer.write_all(&frame.to_bytes()).is_err() {
            return false;
        }
    }

    // The response is complete: send it now, the client is waiting.
    if writer.flush().is_err() {
        return false;
    }

    true
}

fn react(decision: Decision, last_stream_id: u32, mut writer: impl Write) -> bool {
    match decision {
        Decision::Reply(frame) => writer.write_all(&frame.to_bytes()).is_ok(),
        Decision::Continue => true,
        Decision::Close(code) => {
            fail_connection(&mut writer, last_stream_id, code);
            false
        }
    }
}

pub fn serve(
    mut reader: impl BufRead, // reader
    mut writer: impl Write,   // writer
    handler: impl Fn(&Request) -> Response,
) {
    let local_settings = Settings::default();
    let mut peer_settings = Settings::default();

    let empty_frame = Frame::new(FrameType::Settings, Flags::empty(), 0, []);

    if !send_frames(&mut writer, &[empty_frame]) {
        return;
    };

    // RFC 9113 §3.4: the client preface must be followed by a SETTINGS frame.
    match read_frame(&mut reader, local_settings.max_frame_size()) {
        Ok(Some(frame)) if frame.frame_type() == FrameType::Settings => {
            if !react(handle_frame(frame, &mut peer_settings), 0, &mut writer) {
                return;
            }
        }
        Ok(Some(_)) => {
            fail_connection(&mut writer, 0, ErrorCode::ProtocolError);
            return;
        }
        Ok(None) => return,
        Err(error) => {
            if let Some(code) = error.code() {
                fail_connection(&mut writer, 0, code);
            }
            return;
        }
    };

    let mut assembler = Assembler::new();

    loop {
        match read_frame(&mut reader, local_settings.max_frame_size()) {
            Ok(Some(frame)) => {
                match assembler.on_frame(&frame) {
                    Ok(Some(assembled)) => {
                        let (stream_id, request) = assembled.into_parts();
                        let response = handler(&request);

                        let frames =
                            serialize_response(stream_id, response, peer_settings.max_frame_size());

                        if !send_frames(&mut writer, &frames) {
                            return;
                        };
                    }
                    Ok(None) => (),
                    Err(error) => {
                        if error.is_connection_error() {
                            eprintln!("http2: {error}");
                            fail_connection(&mut writer, assembler.last_stream_id(), error.code());
                            return;
                        }

                        let frame_rst_stream = rst_stream(frame.stream_id(), error.code());
                        if !send_frames(&mut writer, &[frame_rst_stream]) {
                            return;
                        };
                    }
                };

                if !react(
                    handle_frame(frame, &mut peer_settings),
                    assembler.last_stream_id(),
                    &mut writer,
                ) {
                    return;
                }
            }
            Err(error) => {
                if let Some(code) = error.code() {
                    fail_connection(&mut writer, assembler.last_stream_id(), code);
                }
                break;
            }
            Ok(None) => break,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http2::frame::{Flags, Frame, FrameType, read_frame};
    use std::io::Cursor;

    // Runs a whole connection against `input` (what follows the preface) and
    // returns every frame the server wrote.
    fn run(input: Vec<Frame>) -> Vec<Frame> {
        let bytes: Vec<u8> = input.iter().flat_map(Frame::to_bytes).collect();
        run_bytes(bytes)
    }

    fn run_bytes(bytes: Vec<u8>) -> Vec<Frame> {
        let mut output = Vec::new();
        serve(Cursor::new(bytes), &mut output, |_: &Request| {
            Response::not_found()
        });

        let mut reader = Cursor::new(output);
        let mut frames = Vec::new();
        while let Some(frame) = read_frame(&mut reader, u32::MAX).unwrap() {
            frames.push(frame);
        }
        frames
    }

    fn settings(payload: impl Into<Vec<u8>>) -> Frame {
        Frame::new(FrameType::Settings, Flags::empty(), 0, payload)
    }

    fn settings_ack() -> Frame {
        Frame::new(FrameType::Settings, Flags::ACK, 0, [])
    }

    // Our own preface. Empty for now: the RFC defaults suit us.
    fn server_settings() -> Frame {
        settings([])
    }

    fn ping(payload: [u8; 8]) -> Frame {
        Frame::new(FrameType::Ping, Flags::empty(), 0, payload)
    }

    fn ping_ack(payload: [u8; 8]) -> Frame {
        Frame::new(FrameType::Ping, Flags::ACK, 0, payload)
    }

    fn client_goaway() -> Frame {
        // last stream id (4 bytes) + NO_ERROR (4 bytes)
        Frame::new(FrameType::GoAway, Flags::empty(), 0, [0u8; 8])
    }

    // No stream was opened in these tests: the last stream id is always 0.
    fn server_goaway(code: ErrorCode) -> Frame {
        goaway(0, code, &[])
    }

    // --- Connection preface (RFC 9113 §3.4) ---

    // The server preface does not wait for the client: it is the first frame we write.
    #[test]
    fn server_settings_are_sent_even_if_the_client_says_nothing() {
        assert_eq!(run(vec![]), vec![server_settings()]);
    }

    #[test]
    fn client_settings_are_acknowledged() {
        assert_eq!(
            run(vec![settings([])]),
            vec![server_settings(), settings_ack()]
        );
    }

    // SETTINGS_MAX_FRAME_SIZE = 32_768
    #[test]
    fn client_settings_with_parameters_are_acknowledged() {
        let payload = [0x00, 0x05, 0x00, 0x00, 0x80, 0x00];

        assert_eq!(
            run(vec![settings(payload)]),
            vec![server_settings(), settings_ack()]
        );
    }

    // RFC 9113 §6.5: an ACK is never acknowledged, otherwise both sides would loop.
    #[test]
    fn client_ack_is_not_acknowledged() {
        assert_eq!(
            run(vec![settings([]), settings_ack()]),
            vec![server_settings(), settings_ack()]
        );
    }

    // RFC 9113 §3.4: the client preface MUST be followed by a SETTINGS frame,
    // otherwise it is a connection error of type PROTOCOL_ERROR.
    #[test]
    fn first_client_frame_must_be_settings() {
        assert_eq!(
            run(vec![ping([1; 8]), settings([])]),
            vec![server_settings(), server_goaway(ErrorCode::ProtocolError)]
        );
    }

    // The size check happens before the type is known: FRAME_SIZE_ERROR, then stop.
    #[test]
    fn first_client_frame_too_large_closes_the_connection() {
        let big = Frame::new(FrameType::Settings, Flags::empty(), 0, vec![0; 16_386]);

        assert_eq!(
            run(vec![big, settings([])]),
            vec![server_settings(), server_goaway(ErrorCode::FrameSizeError)]
        );
    }

    // --- SETTINGS validation (RFC 9113 §6.5) ---

    // Settings apply to the whole connection: stream id must be 0 (PROTOCOL_ERROR).
    #[test]
    fn settings_on_a_stream_close_the_connection() {
        let frame = Frame::new(FrameType::Settings, Flags::empty(), 1, []);

        assert_eq!(
            run(vec![frame]),
            vec![server_settings(), server_goaway(ErrorCode::ProtocolError)]
        );
    }

    // An ACK with a payload is a FRAME_SIZE_ERROR.
    #[test]
    fn settings_ack_with_a_payload_closes_the_connection() {
        let ack = Frame::new(FrameType::Settings, Flags::ACK, 0, [0x00, 0x01, 0, 0, 0, 0]);

        assert_eq!(
            run(vec![settings([]), ack, ping([1; 8])]),
            vec![
                server_settings(),
                settings_ack(),
                server_goaway(ErrorCode::FrameSizeError)
            ]
        );
    }

    // ENABLE_PUSH = 2: invalid settings must not be acknowledged.
    #[test]
    fn invalid_settings_are_not_acknowledged() {
        let payload = [0x00, 0x02, 0x00, 0x00, 0x00, 0x02];

        assert_eq!(
            run(vec![settings(payload)]),
            vec![server_settings(), server_goaway(ErrorCode::ProtocolError)]
        );
    }

    // --- PING (RFC 9113 §6.7) ---

    #[test]
    fn ping_is_answered_with_the_same_payload() {
        let payload = [1, 2, 3, 4, 5, 6, 7, 8];

        assert_eq!(
            run(vec![settings([]), ping(payload)]),
            vec![server_settings(), settings_ack(), ping_ack(payload)]
        );
    }

    #[test]
    fn each_ping_gets_its_own_answer_in_order() {
        assert_eq!(
            run(vec![settings([]), ping([1; 8]), ping([2; 8])]),
            vec![
                server_settings(),
                settings_ack(),
                ping_ack([1; 8]),
                ping_ack([2; 8])
            ]
        );
    }

    // A PING ACK is the answer to one of our pings: it must not be answered.
    #[test]
    fn ping_ack_is_not_answered() {
        assert_eq!(
            run(vec![settings([]), ping_ack([1; 8])]),
            vec![server_settings(), settings_ack()]
        );
    }

    // PING belongs to the connection: a stream id is a PROTOCOL_ERROR.
    #[test]
    fn ping_on_a_stream_closes_the_connection() {
        let frame = Frame::new(FrameType::Ping, Flags::empty(), 1, [1; 8]);

        assert_eq!(
            run(vec![settings([]), frame, ping([2; 8])]),
            vec![
                server_settings(),
                settings_ack(),
                server_goaway(ErrorCode::ProtocolError)
            ]
        );
    }

    // A payload other than 8 bytes is a FRAME_SIZE_ERROR.
    #[test]
    fn ping_of_the_wrong_size_closes_the_connection() {
        let frame = Frame::new(FrameType::Ping, Flags::empty(), 0, [1; 7]);

        assert_eq!(
            run(vec![settings([]), frame, ping([2; 8])]),
            vec![
                server_settings(),
                settings_ack(),
                server_goaway(ErrorCode::FrameSizeError)
            ]
        );
    }

    // --- Closing ---

    // RFC 9113 §6.8: after GOAWAY the client sends nothing new; we stop reading.
    // Answering with our own GOAWAY(NO_ERROR) is allowed: both sides may send one.
    #[test]
    fn goaway_ends_the_connection() {
        assert_eq!(
            run(vec![settings([]), client_goaway(), ping([1; 8])]),
            vec![
                server_settings(),
                settings_ack(),
                server_goaway(ErrorCode::NoError)
            ]
        );
    }

    // RFC 9113 §4.1: unknown frame types MUST be ignored, the connection goes on.
    #[test]
    fn unknown_frames_are_ignored() {
        let unknown = Frame::new(FrameType::Unknown(0xbe), Flags::empty(), 0, [1, 2, 3]);

        assert_eq!(
            run(vec![settings([]), unknown, ping([1; 8])]),
            vec![server_settings(), settings_ack(), ping_ack([1; 8])]
        );
    }

    // A truncated frame ends the connection without panicking.
    #[test]
    fn truncated_frame_ends_the_connection() {
        let mut bytes = settings([]).to_bytes();
        bytes.extend_from_slice(&[0x00, 0x00, 0x08, 0x06, 0x00]);

        assert_eq!(run_bytes(bytes), vec![server_settings(), settings_ack()]);
    }

    // Our receive limit is the one we announced (default 16_384), not the client's.
    // RFC 9113 §4.2: FRAME_SIZE_ERROR, and nothing after it is read.
    #[test]
    fn frame_larger_than_our_max_closes_the_connection() {
        let big = Frame::new(FrameType::Unknown(0xbe), Flags::empty(), 0, vec![0; 16_385]);

        assert_eq!(
            run(vec![settings([]), big, ping([1; 8])]),
            vec![
                server_settings(),
                settings_ack(),
                server_goaway(ErrorCode::FrameSizeError)
            ]
        );
    }

    // The peer's SETTINGS_MAX_FRAME_SIZE (32_768 here) only bounds what WE send.
    // What we read stays bounded by what we announced: the default 16_384.
    #[test]
    fn peer_max_frame_size_does_not_raise_our_receive_limit() {
        let big_frames = [0x00, 0x05, 0x00, 0x00, 0x80, 0x00];
        let big = Frame::new(FrameType::Unknown(0xbe), Flags::empty(), 0, vec![0; 20_000]);

        assert_eq!(
            run(vec![settings(big_frames), big, ping([1; 8])]),
            vec![
                server_settings(),
                settings_ack(),
                server_goaway(ErrorCode::FrameSizeError)
            ]
        );
    }

    // --- Responses (step 7) ---

    use crate::http::{headers::Headers, status::StatusCode};
    use crate::http2::hpack::encoder::encode;

    // Same as `run`, with the handler under test instead of `not_found`.
    fn run_with(input: Vec<Frame>, handler: impl Fn(&Request) -> Response) -> Vec<Frame> {
        let bytes: Vec<u8> = input.iter().flat_map(Frame::to_bytes).collect();
        let mut output = Vec::new();
        serve(Cursor::new(bytes), &mut output, handler);

        let mut reader = Cursor::new(output);
        let mut frames = Vec::new();
        while let Some(frame) = read_frame(&mut reader, u32::MAX).unwrap() {
            frames.push(frame);
        }
        frames
    }

    fn echo_path(request: &Request) -> Response {
        Response::new(
            StatusCode::Ok,
            Headers::default(),
            request.path().to_owned(),
        )
    }

    fn echo_body(request: &Request) -> Response {
        Response::new(
            StatusCode::Ok,
            Headers::default(),
            request.body().to_owned(),
        )
    }

    fn request(stream_id: u32, flags: Flags, method: &str, path: &str) -> Frame {
        let block = encode(&[(":method", method), (":scheme", "http"), (":path", path)]);
        Frame::new(FrameType::Headers, flags, stream_id, block)
    }

    fn get(stream_id: u32, path: &str) -> Frame {
        request(
            stream_id,
            Flags::END_HEADERS | Flags::END_STREAM,
            "GET",
            path,
        )
    }

    fn data(stream_id: u32, flags: Flags, payload: &[u8]) -> Frame {
        Frame::new(FrameType::Data, flags, stream_id, payload)
    }

    // What follows the connection preface in every response test.
    fn responses(frames: Vec<Frame>) -> Vec<Frame> {
        assert_eq!(frames[..2], [server_settings(), settings_ack()]);
        frames.into_iter().skip(2).collect()
    }

    #[test]
    fn get_is_answered_on_its_stream() {
        let frames = responses(run_with(vec![settings([]), get(1, "/echo/abc")], echo_path));

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].frame_type(), FrameType::Headers);
        assert_eq!(frames[0].flags(), Flags::END_HEADERS);
        assert_eq!(frames[0].stream_id(), 1);
        assert_eq!(frames[1], data(1, Flags::END_STREAM, b"/echo/abc"));
    }

    #[test]
    fn post_body_reaches_the_handler() {
        let input = vec![
            settings([]),
            request(1, Flags::END_HEADERS, "POST", "/files/a"),
            data(1, Flags::empty(), b"ab"),
            data(1, Flags::END_STREAM, b"c"),
        ];
        let frames = responses(run_with(input, echo_body));

        assert_eq!(frames[1], data(1, Flags::END_STREAM, b"abc"));
    }

    // Nothing is answered before END_STREAM: the request is not complete yet.
    #[test]
    fn no_response_before_end_stream() {
        let input = vec![
            settings([]),
            request(1, Flags::END_HEADERS, "POST", "/files/a"),
            data(1, Flags::empty(), b"ab"),
        ];

        assert_eq!(responses(run_with(input, echo_body)), vec![]);
    }

    // Until step 10, streams are handled one after the other, in arrival order.
    #[test]
    fn sequential_requests_are_answered_in_order() {
        let input = vec![settings([]), get(1, "/one"), get(3, "/three")];
        let frames = responses(run_with(input, echo_path));

        let data: Vec<_> = frames
            .iter()
            .filter(|frame| frame.frame_type() == FrameType::Data)
            .collect();
        assert_eq!(data[0], &self::data(1, Flags::END_STREAM, b"/one"));
        assert_eq!(data[1], &self::data(3, Flags::END_STREAM, b"/three"));
    }

    // We send what the PEER can receive: SETTINGS_MAX_FRAME_SIZE = 32_768 here.
    #[test]
    fn response_uses_the_peer_max_frame_size() {
        let big_frames = [0x00, 0x05, 0x00, 0x00, 0x80, 0x00];
        let input = vec![settings(big_frames), get(1, "/big")];
        let frames = responses(run_with(input, |_: &Request| {
            Response::new(StatusCode::Ok, Headers::default(), vec![0u8; 40_000])
        }));

        let sizes: Vec<_> = frames
            .iter()
            .filter(|frame| frame.frame_type() == FrameType::Data)
            .map(|frame| frame.payload().len())
            .collect();
        assert_eq!(sizes, vec![32_768, 7_232]);
    }

    // A malformed request (no :path) is a stream error: the handler is not called,
    // the stream is reset with PROTOCOL_ERROR and the connection goes on.
    #[test]
    fn malformed_request_is_reset_and_the_connection_goes_on() {
        let input = vec![
            settings([]),
            malformed(1, Flags::END_HEADERS | Flags::END_STREAM),
            ping([1; 8]),
        ];

        assert_eq!(
            responses(run_with(input, |_: &Request| panic!("handler called"))),
            vec![rst_stream(1, ErrorCode::ProtocolError), ping_ack([1; 8])]
        );
    }

    // --- Errors (step 8, RFC 9113 §5.4) ---

    // A request without :path: always a stream error.
    fn malformed(stream_id: u32, flags: Flags) -> Frame {
        let block = encode(&[(":method", "GET"), (":scheme", "http")]);
        Frame::new(FrameType::Headers, flags, stream_id, block)
    }

    fn is_rst_stream(frame: &&Frame) -> bool {
        frame.frame_type() == FrameType::RstStream
    }

    // The RST_STREAM names the stream that failed, not the last one opened.
    // Stream 1 fails at END_STREAM (content-length mismatch) after stream 3 was opened.
    #[test]
    fn rst_stream_targets_the_failing_stream() {
        let post = Frame::new(
            FrameType::Headers,
            Flags::END_HEADERS,
            1,
            encode(&[
                (":method", "POST"),
                (":scheme", "http"),
                (":path", "/files/a"),
                ("content-length", "5"),
            ]),
        );
        let input = vec![
            settings([]),
            post,
            get(3, "/three"),
            data(1, Flags::END_STREAM, b"hi"),
        ];
        let frames = responses(run_with(input, echo_path));

        assert_eq!(
            frames.last(),
            Some(&rst_stream(1, ErrorCode::ProtocolError))
        );
    }

    // RFC 9113 §5.1: after sending RST_STREAM, frames still in flight on that
    // stream MUST be ignored. Here the body arrives after the headers were rejected.
    #[test]
    fn data_after_a_reset_is_ignored() {
        let input = vec![
            settings([]),
            malformed(1, Flags::END_HEADERS),
            data(1, Flags::empty(), b"ab"),
            data(1, Flags::END_STREAM, b"c"),
            ping([1; 8]),
        ];

        assert_eq!(
            responses(run_with(input, |_: &Request| panic!("handler called"))),
            vec![rst_stream(1, ErrorCode::ProtocolError), ping_ack([1; 8])]
        );
    }

    // DATA after END_STREAM is STREAM_CLOSED (§5.1, half-closed remote).
    // A client that keeps sending gets a single RST_STREAM, not one per frame.
    #[test]
    fn stream_closed_is_reset_only_once() {
        let input = vec![
            settings([]),
            get(1, "/one"),
            data(1, Flags::END_STREAM, b"late"),
            data(1, Flags::END_STREAM, b"later"),
            ping([1; 8]),
        ];
        let frames = responses(run_with(input, echo_path));

        let resets: Vec<_> = frames.iter().filter(is_rst_stream).collect();
        assert_eq!(resets, vec![&rst_stream(1, ErrorCode::StreamClosed)]);
        assert_eq!(frames.last(), Some(&ping_ack([1; 8])));
    }

    // A reset stream does not stop the others: stream 3 is still answered.
    #[test]
    fn requests_after_a_reset_are_answered() {
        let input = vec![
            settings([]),
            malformed(1, Flags::END_HEADERS | Flags::END_STREAM),
            get(3, "/three"),
        ];
        let frames = responses(run_with(input, echo_path));

        assert_eq!(frames[0], rst_stream(1, ErrorCode::ProtocolError));
        assert_eq!(frames.last(), Some(&data(3, Flags::END_STREAM, b"/three")));
    }

    // RFC 9113 §6.8: GOAWAY carries the highest stream id the server processed,
    // so the client knows which requests it can safely retry.
    #[test]
    fn goaway_reports_the_last_processed_stream() {
        let ping_on_a_stream = Frame::new(FrameType::Ping, Flags::empty(), 1, [1; 8]);
        let input = vec![
            settings([]),
            get(1, "/one"),
            get(3, "/three"),
            ping_on_a_stream,
        ];
        let frames = responses(run_with(input, echo_path));

        assert_eq!(
            frames.last(),
            Some(&goaway(3, ErrorCode::ProtocolError, &[]))
        );
    }

    // A connection error raised by the assembler ends the connection: the request
    // sent after it is never answered.
    #[test]
    fn nothing_is_read_after_a_connection_error() {
        let stray_continuation = Frame::new(FrameType::Continuation, Flags::END_HEADERS, 1, []);
        let input = vec![
            settings([]),
            get(1, "/one"),
            stray_continuation,
            get(3, "/three"),
        ];
        let frames = responses(run_with(input, echo_path));

        assert_eq!(
            frames.last(),
            Some(&goaway(1, ErrorCode::ProtocolError, &[]))
        );
        assert!(frames.iter().all(|frame| frame.stream_id() != 3));
    }

    // RFC 9113 §4.3: an HPACK error is always a connection error (COMPRESSION_ERROR).
    #[test]
    fn hpack_error_closes_the_connection() {
        let undecodable = Frame::new(
            FrameType::Headers,
            Flags::END_HEADERS | Flags::END_STREAM,
            1,
            [0xff],
        );
        let input = vec![settings([]), undecodable, ping([1; 8])];
        let frames = responses(run_with(input, echo_path));

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_type(), FrameType::GoAway);
        assert_eq!(frames[0].payload()[4..8], [0x00, 0x00, 0x00, 0x09]);
    }

    // --- RST_STREAM and GOAWAY received from the client (step 8.6) ---

    // The client cancels a request while its body is still in flight: the body
    // is ignored, nothing is answered, and we never reset a stream the client reset.
    #[test]
    fn client_reset_cancels_the_request() {
        let input = vec![
            settings([]),
            request(1, Flags::END_HEADERS, "POST", "/files/a"),
            rst_stream(1, ErrorCode::Cancel),
            data(1, Flags::END_STREAM, b"abc"),
            ping([1; 8]),
        ];

        assert_eq!(
            responses(run_with(input, echo_body)),
            vec![ping_ack([1; 8])]
        );
    }

    #[test]
    fn client_reset_does_not_affect_the_other_streams() {
        let input = vec![
            settings([]),
            request(1, Flags::END_HEADERS, "POST", "/files/a"),
            rst_stream(1, ErrorCode::Cancel),
            get(3, "/three"),
        ];
        let frames = responses(run_with(input, echo_path));

        assert!(frames.iter().all(|frame| frame.stream_id() != 1));
        assert_eq!(frames.last(), Some(&data(3, Flags::END_STREAM, b"/three")));
    }

    // The response and the client's RST_STREAM crossed on the wire: the stream
    // is already closed on our side, which is not an error (§5.1).
    #[test]
    fn client_reset_on_a_closed_stream_is_ignored() {
        let input = vec![
            settings([]),
            get(1, "/one"),
            rst_stream(1, ErrorCode::Cancel),
            ping([1; 8]),
        ];
        let frames = responses(run_with(input, echo_path));

        assert!(!frames.iter().any(|frame| is_rst_stream(&frame)));
        assert_eq!(frames.last(), Some(&ping_ack([1; 8])));
    }

    // §6.4: RST_STREAM on stream 0 is a connection error (PROTOCOL_ERROR).
    #[test]
    fn client_reset_on_stream_0_closes_the_connection() {
        let input = vec![settings([]), rst_stream(0, ErrorCode::Cancel), ping([1; 8])];

        assert_eq!(
            responses(run_with(input, echo_path)),
            vec![server_goaway(ErrorCode::ProtocolError)]
        );
    }

    // §6.4: RST_STREAM on an idle stream (never opened) is a connection error.
    #[test]
    fn client_reset_on_an_idle_stream_closes_the_connection() {
        let input = vec![settings([]), rst_stream(1, ErrorCode::Cancel), ping([1; 8])];

        assert_eq!(
            responses(run_with(input, echo_path)),
            vec![server_goaway(ErrorCode::ProtocolError)]
        );
    }

    // §6.4: a RST_STREAM payload other than 4 octets is FRAME_SIZE_ERROR.
    #[test]
    fn client_reset_of_the_wrong_size_closes_the_connection() {
        let short_reset = Frame::new(FrameType::RstStream, Flags::empty(), 1, [0x00; 3]);
        let input = vec![
            settings([]),
            request(1, Flags::END_HEADERS, "POST", "/files/a"),
            short_reset,
            ping([1; 8]),
        ];

        assert_eq!(
            responses(run_with(input, echo_path)),
            vec![goaway(1, ErrorCode::FrameSizeError, &[])]
        );
    }

    // §6.8: GOAWAY applies to the connection, it MUST be sent on stream 0.
    #[test]
    fn client_goaway_on_a_stream_closes_the_connection() {
        let goaway_on_a_stream = Frame::new(FrameType::GoAway, Flags::empty(), 1, [0x00; 8]);
        let input = vec![settings([]), goaway_on_a_stream, ping([1; 8])];

        assert_eq!(
            responses(run_with(input, echo_path)),
            vec![server_goaway(ErrorCode::ProtocolError)]
        );
    }

    // Last-Stream-ID and Error Code are mandatory: a shorter GOAWAY is FRAME_SIZE_ERROR.
    #[test]
    fn client_goaway_shorter_than_8_bytes_closes_the_connection() {
        let short_goaway = Frame::new(FrameType::GoAway, Flags::empty(), 0, [0x00; 7]);
        let input = vec![settings([]), short_goaway, ping([1; 8])];

        assert_eq!(
            responses(run_with(input, echo_path)),
            vec![server_goaway(ErrorCode::FrameSizeError)]
        );
    }
}
