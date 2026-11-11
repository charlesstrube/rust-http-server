pub mod assembler;
pub mod connection;
pub mod frame;
pub mod hpack;
pub mod serialize;
pub mod settings;

use std::io::{BufRead, Write};

use crate::{
    http::{request::Request, response::Response},
    http2::{
        assembler::{AssembleError, Assembler},
        connection::{Decision, handle_frame},
        frame::{Flags, Frame, FrameType, read_frame},
        serialize::serialize_response,
        settings::Settings,
    },
};

pub(crate) const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub fn react(decision: Decision, mut writer: impl Write) -> bool {
    match decision {
        Decision::Reply(frame) => writer.write_all(&frame.to_bytes()).is_ok(),
        Decision::Continue => true,
        Decision::Close => false,
    }
}

fn is_connection_error(error: AssembleError) -> bool {
    !matches!(
        error,
        AssembleError::StreamClosed | AssembleError::MalformedRequest
    )
}

pub fn serve(
    mut reader: impl BufRead, // reader
    mut writer: impl Write,   // writer
    handler: impl Fn(&Request) -> Response,
) {
    let local_settings = Settings::default();
    let mut peer_settings = Settings::default();

    let empty_frame = Frame::new(FrameType::Settings, Flags::empty(), 0, []);

    if writer.write_all(&empty_frame.to_bytes()).is_err() {
        return;
    };

    // RFC 9113 §3.4: the client preface must be followed by a SETTINGS frame.
    match read_frame(&mut reader, local_settings.max_frame_size()) {
        Ok(Some(frame)) if frame.frame_type() == FrameType::Settings => {
            if !react(handle_frame(frame, &mut peer_settings), &mut writer) {
                return;
            }
        }
        _ => return, // EOF, read error, or another frame type
    };

    let mut assembler = Assembler::new();

    while let Ok(Some(frame)) = read_frame(&mut reader, peer_settings.max_frame_size()) {
        let continue_connection = match assembler.on_frame(&frame) {
            Ok(Some(assembled)) => {
                let (stream_id, request) = assembled.into_parts();
                let response = handler(&request);

                let frames =
                    serialize_response(stream_id, response, peer_settings.max_frame_size());

                for frame in frames.iter() {
                    if !writer.write_all(&frame.to_bytes()).is_ok() {
                        return;
                    }
                }

                true
            }
            Ok(None) => true,
            Err(error) => {
                println!("http2: {error}");
                !is_connection_error(error)
            }
        };

        if continue_connection && !react(handle_frame(frame, &mut peer_settings), &mut writer) {
            return;
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

    fn goaway() -> Frame {
        // last stream id (4 bytes) + NO_ERROR (4 bytes)
        Frame::new(FrameType::GoAway, Flags::empty(), 0, [0u8; 8])
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

    // RFC 9113 §3.4: the client preface MUST be followed by a SETTINGS frame.
    // Step 8 will add a GOAWAY(PROTOCOL_ERROR) before closing.
    #[test]
    fn first_client_frame_must_be_settings() {
        assert_eq!(
            run(vec![ping([1; 8]), settings([])]),
            vec![server_settings()]
        );
    }

    // --- SETTINGS validation (RFC 9113 §6.5) ---

    // Settings apply to the whole connection: stream id must be 0 (PROTOCOL_ERROR).
    #[test]
    fn settings_on_a_stream_close_the_connection() {
        let frame = Frame::new(FrameType::Settings, Flags::empty(), 1, []);

        assert_eq!(run(vec![frame]), vec![server_settings()]);
    }

    // An ACK with a payload is a FRAME_SIZE_ERROR.
    #[test]
    fn settings_ack_with_a_payload_closes_the_connection() {
        let ack = Frame::new(FrameType::Settings, Flags::ACK, 0, [0x00, 0x01, 0, 0, 0, 0]);

        assert_eq!(
            run(vec![settings([]), ack, ping([1; 8])]),
            vec![server_settings(), settings_ack()]
        );
    }

    // ENABLE_PUSH = 2: invalid settings must not be acknowledged.
    #[test]
    fn invalid_settings_are_not_acknowledged() {
        let payload = [0x00, 0x02, 0x00, 0x00, 0x00, 0x02];

        assert_eq!(run(vec![settings(payload)]), vec![server_settings()]);
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
            vec![server_settings(), settings_ack()]
        );
    }

    // A payload other than 8 bytes is a FRAME_SIZE_ERROR.
    #[test]
    fn ping_of_the_wrong_size_closes_the_connection() {
        let frame = Frame::new(FrameType::Ping, Flags::empty(), 0, [1; 7]);

        assert_eq!(
            run(vec![settings([]), frame, ping([2; 8])]),
            vec![server_settings(), settings_ack()]
        );
    }

    // --- Closing ---

    // RFC 9113 §6.8: after GOAWAY the client sends nothing new; we stop reading.
    #[test]
    fn goaway_ends_the_connection() {
        assert_eq!(
            run(vec![settings([]), goaway(), ping([1; 8])]),
            vec![server_settings(), settings_ack()]
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
    #[test]
    fn frame_larger_than_our_max_closes_the_connection() {
        let big = Frame::new(FrameType::Unknown(0xbe), Flags::empty(), 0, vec![0; 16_385]);

        assert_eq!(
            run(vec![settings([]), big, ping([1; 8])]),
            vec![server_settings(), settings_ack()]
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
        assert_eq!(*frames[0].stream_id(), 1);
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

    // A malformed request (no :path) is a stream error: the handler is not called and
    // the connection goes on. Step 8 will answer it with RST_STREAM(PROTOCOL_ERROR).
    #[test]
    fn malformed_request_is_not_answered_but_the_connection_goes_on() {
        let block = encode(&[(":method", "GET"), (":scheme", "http")]);
        let malformed = Frame::new(
            FrameType::Headers,
            Flags::END_HEADERS | Flags::END_STREAM,
            1,
            block,
        );
        let input = vec![settings([]), malformed, ping([1; 8])];

        assert_eq!(
            responses(run_with(input, |_: &Request| panic!("handler called"))),
            vec![ping_ack([1; 8])]
        );
    }
}
