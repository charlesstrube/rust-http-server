use crate::{
    http::{headers::Headers, response::Response},
    http2::{
        frame::{Flags, Frame, FrameType},
        hpack::encoder::encode,
    },
};

// Step 7: turns a `Response` into the frames of one stream (RFC 9113 §8.3.2).
//
// Contract, mirror of `http1::serialize::serialize_response`:
//
//     pub fn serialize_response(stream_id: u32, response: Response, max_frame_size: u32) -> Vec<Frame>
//
// - `max_frame_size` is the PEER's SETTINGS_MAX_FRAME_SIZE: what the client can receive.
// - Field order: `:status` first, then the response headers, lowercased.
// - Connection-specific headers are dropped (§8.2.2); `content-length` is set from the final body.
// - HEADERS (+ CONTINUATION if the block is too large), then DATA chunks of at most `max_frame_size`.
// - END_STREAM is on the last frame of the stream: HEADERS if the body is empty, else the last DATA.

fn encode_into_payloads(headers: Headers, payload: &mut Vec<u8>) {
    for (name, value) in headers.iter() {
        // RFC 9113 §8.2.2: connection-specific headers are forbidden
        // clean up forbidden header in http2
        let lowercase_name = name.to_ascii_lowercase();
        if matches!(
            lowercase_name.as_str(),
            "connection" | "keep-alive" | "proxy-connection" | "transfer-encoding" | "upgrade"
        ) {
            continue;
        }

        let bytes = encode(&[(lowercase_name.as_str(), value)]);

        payload.extend_from_slice(&bytes);
    }
}

pub fn serialize_response(stream_id: u32, response: Response, max_frame_size: u32) -> Vec<Frame> {
    let (status_code, mut headers, body) = response.into_parts();

    // pseudo headers
    let code = status_code.code().to_string();
    let bytes = encode(&[(":status".to_ascii_lowercase().as_str(), &code)]);
    let mut serialized_headers = Vec::<u8>::new();
    serialized_headers.extend_from_slice(&bytes);

    headers.set("content-length", &body.len().to_string());
    encode_into_payloads(headers, &mut serialized_headers);

    let header_payloads = serialized_headers.chunks(max_frame_size as usize);
    let header_frame_length = header_payloads.len();
    let mut frames: Vec<Frame> = vec![];

    for (index, payload) in header_payloads.enumerate() {
        let is_first = index == 0;
        let is_last = index + 1 == header_frame_length;

        let mut flags = Flags::empty();
        if body.is_empty() && is_first {
            flags |= Flags::END_STREAM;
        }
        if is_last {
            flags |= Flags::END_HEADERS;
        }

        let frame_type = if is_first {
            FrameType::Headers
        } else {
            FrameType::Continuation
        };

        frames.push(Frame::new(frame_type, flags, stream_id, payload));
    }

    let body_payloads = body.chunks(max_frame_size as usize);
    let body_frame_length = body_payloads.len();

    for (index, payload) in body_payloads.enumerate() {
        let is_last = index + 1 == body_frame_length;
        let mut flags = Flags::empty();
        if is_last {
            flags |= Flags::END_STREAM
        }

        frames.push(Frame::new(FrameType::Data, flags, stream_id, payload));
    }

    frames
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::{headers::Headers, response::Response, status::StatusCode};
    use crate::http2::frame::{Flags, Frame, FrameType};
    use crate::http2::hpack::{decoder::Decoder, encoder::encode};

    const MAX: u32 = 16_384;

    fn empty(status: StatusCode) -> Response {
        Response::new(status, Headers::default(), vec![])
    }

    fn with_headers(fields: &[(&str, &str)], body: impl Into<Vec<u8>>) -> Response {
        let mut headers = Headers::default();
        for (name, value) in fields {
            headers.add(name, value);
        }
        Response::new(StatusCode::Ok, headers, body)
    }

    fn text_plain(body: &str) -> Response {
        with_headers(&[("Content-Type", "text/plain")], body.as_bytes())
    }

    // Non-UTF-8 bytes on purpose: DATA carries opaque bytes (a gzipped body, a file).
    fn binary(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    // Concatenates HEADERS + CONTINUATION payloads, then decodes the block once.
    fn fields(frames: &[Frame]) -> Vec<(String, String)> {
        let block: Vec<u8> = frames
            .iter()
            .filter(|frame| {
                matches!(
                    frame.frame_type(),
                    FrameType::Headers | FrameType::Continuation
                )
            })
            .flat_map(|frame| frame.payload().to_vec())
            .collect();

        Decoder::new(4_096, 1 << 20)
            .decode(&block)
            .expect("the header block does not decode")
            .into_iter()
            .map(|(name, value)| {
                (
                    String::from_utf8(name).unwrap(),
                    String::from_utf8(value).unwrap(),
                )
            })
            .collect()
    }

    fn names(frames: &[Frame]) -> Vec<String> {
        fields(frames).into_iter().map(|(name, _)| name).collect()
    }

    fn header(frames: &[Frame], name: &str) -> Option<String> {
        fields(frames)
            .into_iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value)
    }

    fn data_frames(frames: &[Frame]) -> Vec<&Frame> {
        frames
            .iter()
            .filter(|frame| frame.frame_type() == FrameType::Data)
            .collect()
    }

    fn data_sizes(frames: &[Frame]) -> Vec<usize> {
        data_frames(frames)
            .iter()
            .map(|frame| frame.payload().len())
            .collect()
    }

    fn body(frames: &[Frame]) -> Vec<u8> {
        data_frames(frames)
            .iter()
            .flat_map(|frame| frame.payload().to_vec())
            .collect()
    }

    fn types(frames: &[Frame]) -> Vec<FrameType> {
        frames.iter().map(Frame::frame_type).collect()
    }

    // --- :status (RFC 9113 §8.3.2) ---

    // Three digits, no reason phrase: "404", never "404 Not Found".
    #[test]
    fn status_has_no_reason_phrase() {
        let frames = serialize_response(1, empty(StatusCode::NotFound), MAX);

        assert_eq!(header(&frames, ":status"), Some("404".into()));
    }

    // §8.3: pseudo-headers come before every regular header.
    #[test]
    fn status_is_the_first_field() {
        let frames = serialize_response(1, text_plain("abc"), MAX);

        assert_eq!(names(&frames)[0], ":status");
    }

    // --- Regular headers (RFC 9113 §8.2) ---

    // §8.2.1: an uppercase name makes the response malformed.
    #[test]
    fn header_names_are_lowercased() {
        let frames = serialize_response(1, text_plain("abc"), MAX);

        assert_eq!(header(&frames, "content-type"), Some("text/plain".into()));
        assert!(
            names(&frames)
                .iter()
                .all(|name| *name == name.to_lowercase())
        );
    }

    // Values are not touched: only names are case-insensitive.
    #[test]
    fn header_values_keep_their_case() {
        let response = with_headers(&[("X-Custom", "MiXeD")], vec![]);
        let frames = serialize_response(1, response, MAX);

        assert_eq!(header(&frames, "x-custom"), Some("MiXeD".into()));
    }

    // §8.2.2: these headers describe the TCP connection, which frames now manage.
    #[test]
    fn connection_specific_headers_are_dropped() {
        let response = with_headers(
            &[
                ("Connection", "close"),
                ("Keep-Alive", "timeout=5"),
                ("Proxy-Connection", "keep-alive"),
                ("Transfer-Encoding", "chunked"),
                ("Upgrade", "h2c"),
                ("Content-Type", "text/plain"),
            ],
            "abc",
        );
        let frames = serialize_response(1, response, MAX);

        assert_eq!(
            names(&frames),
            vec![":status", "content-type", "content-length"]
        );
    }

    // Allowed in HTTP/2 and useful to the client, but no longer what delimits the body.
    #[test]
    fn content_length_is_the_body_length() {
        let frames = serialize_response(1, text_plain("abc"), MAX);

        assert_eq!(header(&frames, "content-length"), Some("3".into()));
    }

    // Like HTTP/1.1, the codec owns content-length: a stale value from a handler is replaced.
    #[test]
    fn stale_content_length_is_replaced() {
        let response = with_headers(&[("Content-Length", "999")], "abc");
        let frames = serialize_response(1, response, MAX);

        let lengths: Vec<_> = fields(&frames)
            .into_iter()
            .filter(|(name, _)| name == "content-length")
            .collect();
        assert_eq!(lengths, vec![("content-length".into(), "3".into())]);
    }

    // §8.1.1: a content-length that disagrees with the DATA frames makes the response
    // malformed. An empty body must not keep the handler's stale value.
    #[test]
    fn stale_content_length_is_replaced_on_an_empty_body() {
        let response = with_headers(&[("Content-Length", "999")], vec![]);
        let frames = serialize_response(1, response, MAX);

        let lengths: Vec<_> = fields(&frames)
            .into_iter()
            .filter(|(name, _)| name == "content-length")
            .collect();
        assert_eq!(lengths, vec![("content-length".into(), "0".into())]);
    }

    // --- Frames and flags (RFC 9113 §6.1, §6.2, §8.1) ---

    // No body: the stream ends on HEADERS, no DATA frame at all.
    #[test]
    fn empty_body_ends_the_stream_on_headers() {
        let frames = serialize_response(1, empty(StatusCode::NotFound), MAX);

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_type(), FrameType::Headers);
        assert_eq!(frames[0].flags(), Flags::END_HEADERS | Flags::END_STREAM);
    }

    #[test]
    fn body_goes_in_a_data_frame_that_ends_the_stream() {
        let frames = serialize_response(1, text_plain("abc"), MAX);

        assert_eq!(types(&frames), vec![FrameType::Headers, FrameType::Data]);
        assert_eq!(frames[0].flags(), Flags::END_HEADERS);
        assert_eq!(frames[1].flags(), Flags::END_STREAM);
        assert_eq!(frames[1].payload(), b"abc");
    }

    // Every frame belongs to the request's stream.
    #[test]
    fn every_frame_carries_the_stream_id() {
        let frames = serialize_response(7, text_plain("abc"), MAX);

        assert!(frames.iter().all(|frame| *frame.stream_id() == 7));
    }

    // The walk-through example, byte for byte, with the minimal step 5 encoder.
    #[test]
    fn echo_abc_exact_bytes() {
        let frames = serialize_response(1, text_plain("abc"), MAX);
        let bytes: Vec<u8> = frames.iter().flat_map(Frame::to_bytes).collect();

        let block = encode(&[
            (":status", "200"),
            ("content-type", "text/plain"),
            ("content-length", "3"),
        ]);
        assert_eq!(block.len(), 56);

        let mut expected = vec![0x00, 0x00, 0x38, 0x01, 0x04, 0x00, 0x00, 0x00, 0x01];
        expected.extend_from_slice(&block);
        expected.extend_from_slice(&[0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]);
        expected.extend_from_slice(b"abc");

        assert_eq!(bytes, expected);
    }

    // --- Splitting the body (RFC 9113 §4.2) ---

    // 40_000 = 16_384 + 16_384 + 7_232; END_STREAM only on the last one.
    #[test]
    fn body_is_split_at_max_frame_size() {
        let payload = binary(40_000);
        let frames = serialize_response(1, with_headers(&[], payload.clone()), MAX);

        assert_eq!(data_sizes(&frames), vec![16_384, 16_384, 7_232]);
        let data = data_frames(&frames);
        assert_eq!(data[0].flags(), Flags::empty());
        assert_eq!(data[1].flags(), Flags::empty());
        assert_eq!(data[2].flags(), Flags::END_STREAM);
        assert_eq!(body(&frames), payload);
    }

    // No empty DATA frame at the end when the body fits exactly.
    #[test]
    fn body_of_exactly_max_frame_size_is_one_data_frame() {
        let frames = serialize_response(1, with_headers(&[], binary(16_384)), MAX);

        assert_eq!(data_sizes(&frames), vec![16_384]);
        assert_eq!(data_frames(&frames)[0].flags(), Flags::END_STREAM);
    }

    // The limit is the one given (the peer's), not a hard-coded 16_384.
    #[test]
    fn uses_the_given_max_frame_size() {
        let frames = serialize_response(1, with_headers(&[], binary(40_000)), 32_768);

        assert_eq!(data_sizes(&frames), vec![32_768, 7_232]);
    }

    // --- Large header block (RFC 9113 §6.10) ---

    // A block larger than max_frame_size goes on in CONTINUATION frames.
    // END_STREAM stays on HEADERS: CONTINUATION only has END_HEADERS.
    #[test]
    fn large_header_block_uses_continuation() {
        let big = "x".repeat(20_000);
        let response = with_headers(&[("X-Big", &big)], vec![]);
        let frames = serialize_response(1, response, MAX);

        assert_eq!(
            types(&frames),
            vec![FrameType::Headers, FrameType::Continuation]
        );
        assert_eq!(frames[0].flags(), Flags::END_STREAM);
        assert_eq!(frames[1].flags(), Flags::END_HEADERS);
        assert!(
            frames
                .iter()
                .all(|frame| frame.payload().len() <= MAX as usize)
        );
        assert_eq!(header(&frames, "x-big"), Some(big));
    }

    #[test]
    fn large_header_block_then_body() {
        let big = "x".repeat(20_000);
        let response = with_headers(&[("X-Big", &big)], "abc");
        let frames = serialize_response(1, response, MAX);

        assert_eq!(
            types(&frames),
            vec![FrameType::Headers, FrameType::Continuation, FrameType::Data]
        );
        assert_eq!(frames[0].flags(), Flags::empty());
        assert_eq!(frames[1].flags(), Flags::END_HEADERS);
        assert_eq!(frames[2].flags(), Flags::END_STREAM);
    }
}
