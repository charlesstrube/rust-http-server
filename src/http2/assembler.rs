use std::{collections::HashMap, str::Utf8Error};

use crate::{
    http::{headers::Headers, request::Request},
    http2::{
        frame::{Flags, Frame, FrameType},
        hpack::{HeaderField, HpackError, decoder::Decoder},
    },
};

#[derive(Debug, PartialEq)]
pub enum AssembleError {
    Hpack(HpackError),      // connection error, COMPRESSION_ERROR
    InvalidStreamId,        // connection error, PROTOCOL_ERROR
    InvalidPadding,         // connection error, PROTOCOL_ERROR
    ExpectedContinuation,   // connection error, PROTOCOL_ERROR
    UnexpectedContinuation, // connection error, PROTOCOL_ERROR
    HeaderBlockTooLarge,    // connection error (CONTINUATION flood)
    StreamClosed,           // stream error, STREAM_CLOSED
    MalformedRequest,       // stream error, PROTOCOL_ERROR
}

impl From<HpackError> for AssembleError {
    fn from(value: HpackError) -> Self {
        Self::Hpack(value)
    }
}
impl From<Utf8Error> for AssembleError {
    fn from(_: Utf8Error) -> Self {
        Self::MalformedRequest
    }
}

struct Stream {
    request: Request,              // construite dès la fin du bloc de headers, body vide
    body: Vec<u8>,                 // ce que les DATA ont apporté jusqu'ici
    content_length: Option<usize>, // le content-length annoncé, comparé à END_STREAM
}

#[derive(Clone)]
struct PendingBlock {
    stream_id: u32,   // pour vérifier que les CONTINUATION arrivent sur le bon stream
    end_stream: bool, // END_STREAM vu sur la frame HEADERS, à retenir jusqu'à END_HEADERS
    block: Vec<u8>,   // les fragments concaténés, limités à MAX_HEADER_BLOCK_SIZE
}

pub const MAX_HEADER_BLOCK_SIZE: usize = 16_384;
pub struct Assembler {
    decoder: Decoder,              // un seul par connexion : la table dynamique HPACK
    pending: Option<PendingBlock>, // le « fax » en cours
    streams: HashMap<u32, Stream>, // les requêtes qui attendent leur body
    last_stream_id: u32,
}

fn strip_padding(flags: Flags, payload: &[u8]) -> Result<&[u8], AssembleError> {
    let (pad_len, mut rest) = if flags.contains(Flags::PADDED) {
        let Some((&pad_len, rest)) = payload.split_first() else {
            return Err(AssembleError::InvalidPadding);
        };
        (pad_len, rest)
    } else {
        (0, payload)
    };

    if flags.contains(Flags::PRIORITY) {
        let (_, rest_header) = rest
            .split_at_checked(5)
            .ok_or(AssembleError::InvalidPadding)?;
        rest = rest_header
    }

    if usize::from(pad_len) > rest.len() {
        return Err(AssembleError::InvalidPadding);
    }

    let (rest_unpadded, _) = rest
        .split_at_checked(rest.len() - usize::from(pad_len))
        .ok_or(AssembleError::InvalidPadding)?;

    Ok(rest_unpadded)
}

fn build_request(fields: Vec<HeaderField>) -> Result<(Request, Option<usize>), AssembleError> {
    let mut seen_regular = false;
    let mut method: Option<String> = None;
    let mut scheme: Option<String> = None;
    let mut path: Option<String> = None;
    let mut authority: Option<String> = None;
    let mut content_length = None;
    let mut headers = Headers::default();
    for (name, value) in fields {
        let decoded_name = str::from_utf8(&name)?;
        let decoded_value = str::from_utf8(&value)?;
        if name.iter().any(u8::is_ascii_uppercase) {
            return Err(AssembleError::MalformedRequest);
        }

        match decoded_name.split_at(1) {
            (":", header_name) => match header_name {
                "method" => {
                    if seen_regular || method.is_some() {
                        return Err(AssembleError::MalformedRequest);
                    }
                    method = Some(decoded_value.to_string())
                }
                "scheme" => {
                    if seen_regular || scheme.is_some() {
                        return Err(AssembleError::MalformedRequest);
                    }
                    scheme = Some(decoded_value.to_string())
                }
                "path" => {
                    if seen_regular || path.is_some() || decoded_value.is_empty() {
                        return Err(AssembleError::MalformedRequest);
                    }

                    path = Some(decoded_value.to_string())
                }
                "authority" => {
                    if seen_regular || authority.is_some() {
                        return Err(AssembleError::MalformedRequest);
                    }
                    authority = Some(decoded_value.to_string())
                }
                _ => return Err(AssembleError::MalformedRequest),
            },
            _ => {
                seen_regular = true;
                if matches!(
                    decoded_name,
                    "connection"
                        | "keep-alive"
                        | "proxy-connection"
                        | "transfer-encoding"
                        | "upgrade"
                ) {
                    return Err(AssembleError::MalformedRequest);
                }

                if decoded_name == "te" && decoded_value != "trailers" {
                    return Err(AssembleError::MalformedRequest);
                }

                if decoded_name == "content-length" {
                    match decoded_value.parse::<usize>() {
                        Ok(value) => content_length = Some(value),
                        Err(_) => return Err(AssembleError::MalformedRequest),
                    }
                }
                headers.add(decoded_name, decoded_value);
            }
        }
    }

    let (Some(path_value), Some(method_value), Some(scheme_value)) = (path, method, scheme) else {
        return Err(AssembleError::MalformedRequest);
    };
    if let Some(authority_value) = &authority {
        headers.add("host", authority_value.as_str());
    }

    let Ok(request) = Request::new(path_value.as_str(), method_value.as_str(), headers, "") else {
        return Err(AssembleError::MalformedRequest);
    };

    Ok((request, content_length))
}

fn complete(id: u32, stream: Stream) -> Result<Option<(u32, Request)>, AssembleError> {
    if let Some(content_length) = stream.content_length
        && content_length != stream.body.len()
    {
        return Err(AssembleError::MalformedRequest);
    }

    let mut request = stream.request;

    request.set_body(&stream.body);

    Ok(Some((id, request)))
}

impl Assembler {
    pub fn new() -> Self {
        Self {
            last_stream_id: 0,
            streams: HashMap::new(),
            pending: None,
            decoder: Decoder::new(4096, MAX_HEADER_BLOCK_SIZE),
        }
    }

    fn on_header(&mut self, frame: Frame) -> Result<Option<(u32, Request)>, AssembleError> {
        let stream_id = *frame.stream_id();
        if stream_id % 2 == 0 || stream_id <= self.last_stream_id {
            return Err(AssembleError::InvalidStreamId);
        }

        self.last_stream_id = stream_id;

        let fragment = strip_padding(frame.flags(), frame.payload())?;

        if fragment.len() > MAX_HEADER_BLOCK_SIZE {
            return Err(AssembleError::HeaderBlockTooLarge);
        }

        let pending_block = PendingBlock {
            stream_id,
            block: fragment.to_vec(),
            end_stream: frame.flags().contains(Flags::END_STREAM),
        };

        if frame.flags().contains(Flags::END_HEADERS) {
            self.finish_block(pending_block)
        } else {
            self.pending = Some(pending_block);
            Ok(None)
        }
    }

    fn on_data(&mut self, frame: Frame) -> Result<Option<(u32, Request)>, AssembleError> {
        let current_stream_id = *frame.stream_id();

        if current_stream_id == 0 {
            return Err(AssembleError::InvalidStreamId);
        }

        match self.streams.get_mut(&current_stream_id) {
            None => {
                if current_stream_id > self.last_stream_id {
                    return Err(AssembleError::InvalidStreamId);
                }

                // current_stream_id <= self.last_stream_id
                Err(AssembleError::StreamClosed)
            }
            Some(stream) => {
                let data = strip_padding(frame.flags(), frame.payload())?;

                stream.body.extend_from_slice(data);

                if frame.flags().contains(Flags::END_STREAM) {
                    let stream = self
                        .streams
                        .remove(&current_stream_id)
                        .expect("to remove stream");

                    complete(current_stream_id, stream)
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn finish_block(
        &mut self,
        pending: PendingBlock,
    ) -> Result<Option<(u32, Request)>, AssembleError> {
        let fields = self.decoder.decode(&pending.block)?;
        let (request, content_length) = build_request(fields)?;

        let stream = Stream {
            request,
            content_length,
            body: vec![],
        };
        if pending.end_stream {
            complete(pending.stream_id, stream)
        } else {
            self.streams.insert(pending.stream_id, stream);
            Ok(None)
        }
    }

    fn on_continuation(&mut self, frame: Frame) -> Result<Option<(u32, Request)>, AssembleError> {
        let Some(mut pending) = self.pending.take() else {
            panic!("should continue")
        };

        if frame.frame_type() != FrameType::Continuation || pending.stream_id != *frame.stream_id()
        {
            return Err(AssembleError::ExpectedContinuation);
        }

        if pending.block.len() + frame.payload().len() > MAX_HEADER_BLOCK_SIZE {
            return Err(AssembleError::HeaderBlockTooLarge);
        }

        pending.block.extend_from_slice(frame.payload());
        if frame.flags().contains(Flags::END_HEADERS) {
            self.finish_block(pending)
        } else {
            self.pending = Some(pending);
            Ok(None)
        }
    }

    pub fn on_frame(&mut self, frame: Frame) -> Result<Option<(u32, Request)>, AssembleError> {
        if self.pending.is_some() {
            self.on_continuation(frame)
        } else {
            match frame.frame_type() {
                FrameType::Data => self.on_data(frame),
                FrameType::Headers => self.on_header(frame),
                FrameType::Continuation => Err(AssembleError::UnexpectedContinuation),
                _ => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::{method::Method, request::Request};
    use crate::http2::frame::{Flags, Frame, FrameType};
    use crate::http2::hpack::encoder::encode;

    const GET: &[(&str, &str)] = &[
        (":method", "GET"),
        (":scheme", "http"),
        (":path", "/echo/abc"),
        (":authority", "localhost:4221"),
    ];

    const POST: &[(&str, &str)] = &[
        (":method", "POST"),
        (":scheme", "http"),
        (":path", "/files/a"),
    ];

    fn end_all() -> Flags {
        Flags::END_HEADERS | Flags::END_STREAM
    }

    fn headers(stream_id: u32, flags: Flags, fields: &[(&str, &str)]) -> Frame {
        Frame::new(FrameType::Headers, flags, stream_id, encode(fields))
    }

    fn raw_headers(stream_id: u32, flags: Flags, payload: impl Into<Vec<u8>>) -> Frame {
        Frame::new(FrameType::Headers, flags, stream_id, payload)
    }

    fn continuation(stream_id: u32, flags: Flags, block: &[u8]) -> Frame {
        Frame::new(FrameType::Continuation, flags, stream_id, block)
    }

    fn data(stream_id: u32, flags: Flags, payload: &[u8]) -> Frame {
        Frame::new(FrameType::Data, flags, stream_id, payload)
    }

    fn ping() -> Frame {
        Frame::new(FrameType::Ping, Flags::empty(), 0, [0; 8])
    }

    // Feeds every frame, panics on any error, returns the completed requests in order.
    fn feed(assembler: &mut Assembler, frames: Vec<Frame>) -> Vec<(u32, Request)> {
        frames
            .into_iter()
            .filter_map(|frame| assembler.on_frame(frame).unwrap())
            .collect()
    }

    // Feeds every frame but the last without error, returns the result of the last one.
    fn last(
        assembler: &mut Assembler,
        mut frames: Vec<Frame>,
    ) -> Result<Option<(u32, Request)>, AssembleError> {
        let last = frames.pop().unwrap();
        feed(assembler, frames);
        assembler.on_frame(last)
    }

    fn single(frames: Vec<Frame>) -> (u32, Request) {
        let mut requests = feed(&mut Assembler::new(), frames);
        assert_eq!(requests.len(), 1, "expected exactly one complete request");
        requests.pop().unwrap()
    }

    // GET with every pseudo-header but one, to check each is mandatory.
    fn without(name: &str) -> Vec<(&'static str, &'static str)> {
        GET.iter().copied().filter(|(n, _)| *n != name).collect()
    }

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        GET.iter().chain(extra).copied().collect()
    }

    // --- A request in a single HEADERS frame (RFC 9113 §6.2, §8.1) ---

    #[test]
    fn get_in_a_single_headers_frame() {
        let (stream_id, request) = single(vec![headers(1, end_all(), GET)]);

        assert_eq!(stream_id, 1);
        assert!(matches!(request.method(), Method::Get));
        assert_eq!(request.path(), "/echo/abc");
        assert_eq!(request.body(), b"");
    }

    // RFC 7541 Appendix C.4.1, Huffman-encoded, wrapped in a HEADERS frame.
    #[test]
    fn rfc_7541_c_4_1_request() {
        let block = [
            0x82, 0x86, 0x84, 0x41, 0x8c, 0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab,
            0x90, 0xf4, 0xff,
        ];

        let (_, request) = single(vec![raw_headers(1, end_all(), block)]);

        assert!(matches!(request.method(), Method::Get));
        assert_eq!(request.path(), "/");
        assert_eq!(request.header("host"), Some("www.example.com"));
    }

    // RFC 9113 §8.3.1: handlers read `Host`; HTTP/2 carries it in `:authority`.
    #[test]
    fn authority_becomes_host() {
        let (_, request) = single(vec![headers(1, end_all(), GET)]);

        assert_eq!(request.header("Host"), Some("localhost:4221"));
    }

    #[test]
    fn pseudo_headers_are_not_regular_headers() {
        let fields = with(&[("user-agent", "curl/8.7.1")]);

        let (_, request) = single(vec![headers(1, end_all(), &fields)]);

        assert_eq!(request.header(":path"), None);
        assert_eq!(request.header(":method"), None);
        assert_eq!(request.header("User-Agent"), Some("curl/8.7.1"));
    }

    // Without END_STREAM, a body may follow: the request is not complete yet.
    #[test]
    fn headers_without_end_stream_wait_for_data() {
        let mut assembler = Assembler::new();

        let result = assembler.on_frame(headers(1, Flags::END_HEADERS, POST));

        assert!(matches!(result, Ok(None)));
    }

    // Frames that belong to the connection are not the assembler's business.
    #[test]
    fn connection_frames_are_ignored() {
        let mut assembler = Assembler::new();

        assert!(matches!(assembler.on_frame(ping()), Ok(None)));
    }

    // --- Body (RFC 9113 §6.1) ---

    #[test]
    fn body_spread_over_two_data_frames() {
        let mut assembler = Assembler::new();

        assert!(matches!(
            assembler.on_frame(headers(1, Flags::END_HEADERS, POST)),
            Ok(None)
        ));
        assert!(matches!(
            assembler.on_frame(data(1, Flags::empty(), b"hel")),
            Ok(None)
        ));
        let (stream_id, request) = assembler
            .on_frame(data(1, Flags::END_STREAM, b"lo"))
            .unwrap()
            .unwrap();

        assert_eq!(stream_id, 1);
        assert!(matches!(request.method(), Method::Post));
        assert_eq!(request.body(), b"hello");
    }

    // curl often ends the body with an empty DATA frame carrying END_STREAM.
    #[test]
    fn empty_data_frame_can_end_the_stream() {
        let (_, request) = single(vec![
            headers(1, Flags::END_HEADERS, POST),
            data(1, Flags::empty(), b"abc"),
            data(1, Flags::END_STREAM, b""),
        ]);

        assert_eq!(request.body(), b"abc");
    }

    // A body is raw bytes: nothing forces it to be UTF-8.
    #[test]
    fn body_is_binary() {
        let (_, request) = single(vec![
            headers(1, Flags::END_HEADERS, POST),
            data(1, Flags::END_STREAM, &[0xff, 0x00, 0xfe]),
        ]);

        assert_eq!(request.body(), &[0xff, 0x00, 0xfe]);
    }

    // PADDED: first byte = pad length, then data, then that many padding bytes.
    #[test]
    fn padded_data_drops_the_padding() {
        let (_, request) = single(vec![
            headers(1, Flags::END_HEADERS, POST),
            data(1, Flags::PADDED | Flags::END_STREAM, &[2, b'h', b'i', 0, 0]),
        ]);

        assert_eq!(request.body(), b"hi");
    }

    // Multiplexing: two streams open at once, completed in the order END_STREAM arrives.
    #[test]
    fn data_of_two_streams_can_interleave() {
        let requests = feed(
            &mut Assembler::new(),
            vec![
                headers(1, Flags::END_HEADERS, POST),
                headers(3, Flags::END_HEADERS, POST),
                data(1, Flags::empty(), b"a"),
                data(3, Flags::END_STREAM, b"b"),
                data(1, Flags::END_STREAM, b"a"),
            ],
        );

        let summary: Vec<(u32, &[u8])> = requests.iter().map(|(id, r)| (*id, r.body())).collect();
        assert_eq!(summary, vec![(3, &b"b"[..]), (1, &b"aa"[..])]);
    }

    // RFC 9113 §8.1.1: content-length must match the DATA received (request smuggling).
    #[test]
    fn content_length_mismatch_is_malformed() {
        let fields = [
            (":method", "POST"),
            (":scheme", "http"),
            (":path", "/files/a"),
            ("content-length", "5"),
        ];

        let result = last(
            &mut Assembler::new(),
            vec![
                headers(1, Flags::END_HEADERS, &fields),
                data(1, Flags::END_STREAM, b"hi"),
            ],
        );

        assert!(matches!(result, Err(AssembleError::MalformedRequest)));
    }

    #[test]
    fn matching_content_length_is_accepted() {
        let fields = [
            (":method", "POST"),
            (":scheme", "http"),
            (":path", "/files/a"),
            ("content-length", "2"),
        ];

        let (_, request) = single(vec![
            headers(1, Flags::END_HEADERS, &fields),
            data(1, Flags::END_STREAM, b"hi"),
        ]);

        assert_eq!(request.body(), b"hi");
    }

    // --- Header block: padding, priority, CONTINUATION (RFC 9113 §6.2, §6.10) ---

    #[test]
    fn header_block_split_over_continuation() {
        let block = encode(GET);
        let (first, rest) = block.split_at(5);

        let mut assembler = Assembler::new();
        assert!(matches!(
            assembler.on_frame(raw_headers(1, Flags::END_STREAM, first)),
            Ok(None)
        ));
        let (stream_id, request) = assembler
            .on_frame(continuation(1, Flags::END_HEADERS, rest))
            .unwrap()
            .unwrap();

        assert_eq!(stream_id, 1);
        assert_eq!(request.path(), "/echo/abc");
    }

    // Splits can fall anywhere, even inside a string: decode only once the block is whole.
    #[test]
    fn header_block_split_over_two_continuations() {
        let block = encode(GET);

        let (_, request) = single(vec![
            raw_headers(1, Flags::END_STREAM, &block[..3]),
            continuation(1, Flags::empty(), &block[3..10]),
            continuation(1, Flags::END_HEADERS, &block[10..]),
        ]);

        assert_eq!(request.path(), "/echo/abc");
    }

    #[test]
    fn padded_headers_drop_the_padding() {
        let mut payload = vec![3];
        payload.extend(encode(GET));
        payload.extend([0, 0, 0]);

        let (_, request) = single(vec![raw_headers(1, end_all() | Flags::PADDED, payload)]);

        assert_eq!(request.path(), "/echo/abc");
    }

    // PRIORITY: 5 bytes (E + stream dependency on 31 bits, weight) to skip, deprecated.
    #[test]
    fn priority_fields_are_skipped() {
        // E = 1, dependency = 0, weight = 15
        let mut payload = vec![0x80, 0, 0, 0, 15];
        payload.extend(encode(GET));

        let (_, request) = single(vec![raw_headers(1, end_all() | Flags::PRIORITY, payload)]);

        assert_eq!(request.path(), "/echo/abc");
    }

    // Both flags: pad length, then priority, then the block, then the padding.
    #[test]
    fn padded_and_priority_together() {
        let mut payload = vec![2, 0, 0, 0, 0, 15];
        payload.extend(encode(GET));
        payload.extend([0, 0]);

        let flags = end_all() | Flags::PADDED | Flags::PRIORITY;
        let (_, request) = single(vec![raw_headers(1, flags, payload)]);

        assert_eq!(request.path(), "/echo/abc");
    }

    // RFC 9113 §6.2: padding as long as (or longer than) the rest is a PROTOCOL_ERROR.
    #[test]
    fn padding_longer_than_the_payload_is_an_error() {
        let payload = vec![10, 0x82, 0x86];

        let result = Assembler::new().on_frame(raw_headers(1, end_all() | Flags::PADDED, payload));

        assert!(matches!(result, Err(AssembleError::InvalidPadding)));
    }

    #[test]
    fn data_padding_longer_than_the_payload_is_an_error() {
        let result = last(
            &mut Assembler::new(),
            vec![
                headers(1, Flags::END_HEADERS, POST),
                data(1, Flags::PADDED | Flags::END_STREAM, &[5, b'h']),
            ],
        );

        assert!(matches!(result, Err(AssembleError::InvalidPadding)));
    }

    // RFC 9113 §6.10: while a header block is open, nothing else may arrive, not even a PING.
    #[test]
    fn frame_inside_a_header_block_is_an_error() {
        let block = encode(GET);

        let result = last(
            &mut Assembler::new(),
            vec![raw_headers(1, Flags::END_STREAM, &block[..5]), ping()],
        );

        assert!(matches!(result, Err(AssembleError::ExpectedContinuation)));
    }

    #[test]
    fn continuation_on_another_stream_is_an_error() {
        let block = encode(GET);

        let result = last(
            &mut Assembler::new(),
            vec![
                raw_headers(1, Flags::END_STREAM, &block[..5]),
                continuation(3, Flags::END_HEADERS, &block[5..]),
            ],
        );

        assert!(matches!(result, Err(AssembleError::ExpectedContinuation)));
    }

    #[test]
    fn continuation_without_an_open_block_is_an_error() {
        let result = last(
            &mut Assembler::new(),
            vec![
                headers(1, end_all(), GET),
                continuation(1, Flags::END_HEADERS, &encode(GET)),
            ],
        );

        assert!(matches!(result, Err(AssembleError::UnexpectedContinuation)));
    }

    // CONTINUATION flood (2024): an endless block must not grow memory forever.
    #[test]
    fn header_block_larger_than_the_limit_is_an_error() {
        let result = last(
            &mut Assembler::new(),
            vec![
                raw_headers(1, Flags::END_STREAM, vec![0; MAX_HEADER_BLOCK_SIZE]),
                continuation(1, Flags::empty(), &[0]),
            ],
        );

        assert!(matches!(result, Err(AssembleError::HeaderBlockTooLarge)));
    }

    // RFC 9113 §4.3: a block HPACK cannot decode is a connection error.
    #[test]
    fn undecodable_block_is_an_hpack_error() {
        let result = Assembler::new().on_frame(raw_headers(1, end_all(), [0xff]));

        assert!(matches!(result, Err(AssembleError::Hpack(_))));
    }

    // --- Stream ids (RFC 9113 §5.1.1) ---

    #[test]
    fn even_stream_id_is_an_error() {
        let result = Assembler::new().on_frame(headers(2, end_all(), GET));

        assert!(matches!(result, Err(AssembleError::InvalidStreamId)));
    }

    #[test]
    fn stream_id_zero_is_an_error() {
        let result = Assembler::new().on_frame(headers(0, end_all(), GET));

        assert!(matches!(result, Err(AssembleError::InvalidStreamId)));
    }

    #[test]
    fn decreasing_stream_id_is_an_error() {
        let result = last(
            &mut Assembler::new(),
            vec![headers(3, end_all(), GET), headers(1, end_all(), GET)],
        );

        assert!(matches!(result, Err(AssembleError::InvalidStreamId)));
    }

    #[test]
    fn reused_stream_id_is_an_error() {
        let result = last(
            &mut Assembler::new(),
            vec![headers(1, end_all(), GET), headers(1, end_all(), GET)],
        );

        assert!(matches!(result, Err(AssembleError::InvalidStreamId)));
    }

    // Skipping ids is fine: streams 3 and 5 are implicitly closed (§5.1.1).
    #[test]
    fn stream_ids_may_skip() {
        let requests = feed(
            &mut Assembler::new(),
            vec![headers(1, end_all(), GET), headers(7, end_all(), GET)],
        );

        let ids: Vec<u32> = requests.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![1, 7]);
    }

    // --- Stream states (RFC 9113 §5.1) ---

    // DATA on a stream that was never opened (idle): PROTOCOL_ERROR.
    #[test]
    fn data_on_an_idle_stream_is_an_error() {
        let result = Assembler::new().on_frame(data(1, Flags::END_STREAM, b"hi"));

        assert!(matches!(result, Err(AssembleError::InvalidStreamId)));
    }

    #[test]
    fn data_on_stream_zero_is_an_error() {
        let result = Assembler::new().on_frame(data(0, Flags::END_STREAM, b"hi"));

        assert!(matches!(result, Err(AssembleError::InvalidStreamId)));
    }

    // After END_STREAM the client has nothing more to send (half-closed remote).
    #[test]
    fn data_after_end_stream_is_stream_closed() {
        let result = last(
            &mut Assembler::new(),
            vec![
                headers(1, end_all(), GET),
                data(1, Flags::END_STREAM, b"late"),
            ],
        );

        assert!(matches!(result, Err(AssembleError::StreamClosed)));
    }

    // --- Malformed requests (RFC 9113 §8.1.1, §8.2, §8.3.1): stream errors ---

    fn assert_malformed(fields: &[(&str, &str)]) {
        let result = Assembler::new().on_frame(headers(1, end_all(), fields));

        assert!(
            matches!(result, Err(AssembleError::MalformedRequest)),
            "expected MalformedRequest for {fields:?}"
        );
    }

    #[test]
    fn mandatory_pseudo_headers_are_required() {
        for name in [":method", ":scheme", ":path"] {
            assert_malformed(&without(name));
        }
    }

    // :authority is optional.
    #[test]
    fn authority_is_optional() {
        let (_, request) = single(vec![headers(1, end_all(), &without(":authority"))]);

        assert_eq!(request.path(), "/echo/abc");
    }

    #[test]
    fn empty_path_is_malformed() {
        assert_malformed(&[(":method", "GET"), (":scheme", "http"), (":path", "")]);
    }

    #[test]
    fn duplicated_pseudo_header_is_malformed() {
        assert_malformed(&with(&[(":path", "/other")]));
    }

    // `:status` exists, but only in responses.
    #[test]
    fn unknown_or_response_pseudo_header_is_malformed() {
        assert_malformed(&with(&[(":foo", "bar")]));
        assert_malformed(&with(&[(":status", "200")]));
    }

    #[test]
    fn pseudo_header_after_a_regular_header_is_malformed() {
        assert_malformed(&[
            (":method", "GET"),
            (":scheme", "http"),
            ("user-agent", "curl"),
            (":path", "/"),
        ]);
    }

    // RFC 9113 §8.2.1: field names are lowercase on the wire.
    #[test]
    fn uppercase_header_name_is_malformed() {
        assert_malformed(&with(&[("User-Agent", "curl")]));
    }

    // RFC 9113 §8.2.2: these only mean something for one HTTP/1.1 hop.
    #[test]
    fn connection_specific_headers_are_malformed() {
        for name in [
            "connection",
            "keep-alive",
            "proxy-connection",
            "transfer-encoding",
            "upgrade",
        ] {
            assert_malformed(&with(&[(name, "x")]));
        }
    }

    #[test]
    fn te_is_only_allowed_as_trailers() {
        assert_malformed(&with(&[("te", "gzip")]));

        let (_, request) = single(vec![headers(1, end_all(), &with(&[("te", "trailers")]))]);
        assert_eq!(request.path(), "/echo/abc");
    }

    // A malformed request is a stream error: the connection, and HPACK, carry on.
    // Stream 1 (RFC 7541 C.3.1 without `:path`) is rejected, but it still added
    // `:authority: www.example.com` to the dynamic table (index 62). Stream 3
    // refers to that entry (0xbe = indexed 62): it only works if stream 1's block
    // was decoded anyway.
    #[test]
    fn malformed_request_still_updates_the_dynamic_table() {
        let mut stream_1 = vec![0x82, 0x86, 0x41, 0x0f];
        stream_1.extend_from_slice(b"www.example.com");
        let stream_3 = [0x82, 0x86, 0x84, 0xbe];

        let mut assembler = Assembler::new();
        let first = assembler.on_frame(raw_headers(1, end_all(), stream_1));
        let (stream_id, request) = assembler
            .on_frame(raw_headers(3, end_all(), stream_3))
            .unwrap()
            .unwrap();

        assert!(matches!(first, Err(AssembleError::MalformedRequest)));
        assert_eq!(stream_id, 3);
        assert_eq!(request.header("host"), Some("www.example.com"));
    }
}
