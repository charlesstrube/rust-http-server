use std::collections::{HashMap, HashSet, hash_map::Entry};

use crate::{
    http::{headers::Headers, request::Request},
    http2::{
        error::{ErrorCode, PayloadError, parse_rst_stream},
        frame::{Flags, Frame, FrameType},
        hpack::{
            HeaderField, HpackError, decoder::Decoder, dynamic_table::SETTINGS_HEADER_TABLE_SIZE,
        },
    },
};

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum AssembleError {
    #[error("invalid payload length")]
    InvalidFrameSize(#[from] PayloadError),
    #[error(transparent)]
    Hpack(#[from] HpackError), // connection error, COMPRESSION_ERROR
    #[error("invalid stream id")]
    InvalidStreamId, // connection error, PROTOCOL_ERROR
    #[error("padding or priority fields do not fit in the frame payload")]
    InvalidPadding, // connection error, PROTOCOL_ERROR
    #[error("expected a CONTINUATION frame on the same stream")]
    ExpectedContinuation, // connection error, PROTOCOL_ERROR
    #[error("CONTINUATION frame without an open header block")]
    UnexpectedContinuation, // connection error, PROTOCOL_ERROR
    #[error("header block larger than MAX_HEADER_BLOCK_SIZE")]
    HeaderBlockTooLarge, // connection error (CONTINUATION flood)
    #[error("frame received on a closed stream")]
    StreamClosed, // stream error, STREAM_CLOSED
    #[error("malformed request")]
    MalformedRequest, // stream error, PROTOCOL_ERROR
}

impl AssembleError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::StreamClosed => ErrorCode::StreamClosed,
            Self::HeaderBlockTooLarge => ErrorCode::EnhanceYourCalm,
            Self::Hpack(_) => ErrorCode::CompressionError,
            Self::InvalidFrameSize(_) => ErrorCode::FrameSizeError,
            Self::MalformedRequest
            | Self::ExpectedContinuation
            | Self::InvalidPadding
            | Self::InvalidStreamId
            | Self::UnexpectedContinuation => ErrorCode::ProtocolError,
        }
    }

    pub fn is_connection_error(&self) -> bool {
        !matches!(
            self,
            AssembleError::StreamClosed | AssembleError::MalformedRequest
        )
    }
}

pub struct Assembled {
    stream_id: u32,
    request: Request,
}

impl Assembled {
    pub fn into_parts(self) -> (u32, Request) {
        (self.stream_id, self.request)
    }
}

struct Stream {
    request: Request,              // built when the header block ends, empty body
    body: Vec<u8>,                 // bytes received in DATA frames so far
    content_length: Option<usize>, // announced content-length, checked at END_STREAM
}

struct PendingBlock {
    stream_id: u32,   // CONTINUATION frames must arrive on this stream
    end_stream: bool, // END_STREAM seen on HEADERS, kept until END_HEADERS
    block: Vec<u8>,   // concatenated fragments, at most MAX_HEADER_BLOCK_SIZE
}

pub const MAX_HEADER_BLOCK_SIZE: usize = 16_384;

#[derive(Default)]
pub struct Assembler {
    decoder: Decoder,              // one per connection: owns the HPACK dynamic table
    pending: Option<PendingBlock>, // header block still waiting for END_HEADERS
    streams: HashMap<u32, Stream>, // requests still waiting for their body
    reset: HashSet<u32>,           // failed streams
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

    rest = &rest[..rest.len() - usize::from(pad_len)];

    Ok(rest)
}

fn is_valid_field(name: &[u8], value: &[u8]) -> bool {
    if name
        .iter()
        .any(|n| matches!(n, 0x00..=0x20 | 0x41..=0x5A | 0x7F..=0xFF))
    {
        return false;
    }
    if value
        .iter()
        .any(|byte| matches!(byte, b'\n' | b'\0' | b'\r'))
    {
        return false;
    }

    if matches!(value.first(), Some(b' ' | b'\t')) || matches!(value.last(), Some(b' ' | b'\t')) {
        return false;
    }

    if name.is_empty() {
        return false;
    }

    if name.iter().skip(1).any(|v| matches!(v, 0x3A)) {
        return false;
    }

    true
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
        let decoded_name = str::from_utf8(&name).map_err(|_| AssembleError::MalformedRequest)?;
        let decoded_value = str::from_utf8(&value).map_err(|_| AssembleError::MalformedRequest)?;

        if !is_valid_field(name.as_slice(), value.as_slice()) {
            return Err(AssembleError::MalformedRequest);
        }

        match decoded_name.strip_prefix(':') {
            Some(name) => {
                if seen_regular {
                    return Err(AssembleError::MalformedRequest);
                }
                let slot = match name {
                    "method" => &mut method,
                    "scheme" => &mut scheme,
                    "authority" => &mut authority,
                    "path" if decoded_value.is_empty() => {
                        return Err(AssembleError::MalformedRequest);
                    }
                    "path" => &mut path,
                    _ => return Err(AssembleError::MalformedRequest),
                };

                if slot.is_some() {
                    return Err(AssembleError::MalformedRequest);
                }
                *slot = Some(decoded_value.to_string())
            }
            None => {
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

    let (Some(path_value), Some(method_value), Some(_)) = (path, method, scheme) else {
        return Err(AssembleError::MalformedRequest);
    };
    if let Some(authority_value) = &authority {
        headers.add("host", authority_value.as_str());
    }

    let Ok(request) = Request::new(path_value.as_str(), method_value.as_str(), headers, vec![])
    else {
        return Err(AssembleError::MalformedRequest);
    };

    Ok((request, content_length))
}

fn complete(id: u32, stream: Stream) -> Result<Option<Assembled>, AssembleError> {
    if let Some(content_length) = stream.content_length
        && content_length != stream.body.len()
    {
        return Err(AssembleError::MalformedRequest);
    }

    let mut request = stream.request;

    request.set_body(stream.body);

    Ok(Some(Assembled {
        stream_id: id,
        request,
    }))
}

impl Assembler {
    pub fn new() -> Self {
        Self {
            last_stream_id: 0,
            streams: HashMap::new(),
            pending: None,
            decoder: Decoder::new(SETTINGS_HEADER_TABLE_SIZE, MAX_HEADER_BLOCK_SIZE),
            reset: HashSet::new(),
        }
    }

    pub fn last_stream_id(&self) -> u32 {
        self.last_stream_id
    }

    fn on_header(&mut self, frame: &Frame) -> Result<Option<Assembled>, AssembleError> {
        if frame.stream_id().is_multiple_of(2) || frame.stream_id() <= self.last_stream_id {
            return Err(AssembleError::InvalidStreamId);
        }

        self.last_stream_id = frame.stream_id();

        let fragment = strip_padding(frame.flags(), frame.payload())?;

        if fragment.len() > MAX_HEADER_BLOCK_SIZE {
            return Err(AssembleError::HeaderBlockTooLarge);
        }

        let pending_block = PendingBlock {
            stream_id: frame.stream_id(),
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

    fn on_data(&mut self, frame: &Frame) -> Result<Option<Assembled>, AssembleError> {
        if self.reset.contains(&frame.stream_id()) {
            return Ok(None);
        }

        if frame.stream_id() == 0 {
            return Err(AssembleError::InvalidStreamId);
        };

        match self.streams.entry(frame.stream_id()) {
            Entry::Vacant(_) => {
                if frame.stream_id() > self.last_stream_id {
                    return Err(AssembleError::InvalidStreamId);
                }

                // current_stream_id <= self.last_stream_id
                Err(AssembleError::StreamClosed)
            }
            Entry::Occupied(mut entry) => {
                let stream = entry.get_mut();
                let data = strip_padding(frame.flags(), frame.payload())?;

                stream.body.extend_from_slice(data);

                if frame.flags().contains(Flags::END_STREAM) {
                    complete(frame.stream_id(), entry.remove())
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn finish_block(&mut self, pending: PendingBlock) -> Result<Option<Assembled>, AssembleError> {
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

    fn on_continuation(
        &mut self,
        frame: &Frame,
        mut pending: PendingBlock,
    ) -> Result<Option<Assembled>, AssembleError> {
        if frame.frame_type() != FrameType::Continuation || pending.stream_id != frame.stream_id() {
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

    fn on_rst_stream(&mut self, frame: &Frame) -> Result<Option<Assembled>, AssembleError> {
        // connection error
        if frame.stream_id() == 0 {
            return Err(AssembleError::InvalidStreamId);
        }

        // ignoring for now
        let _code = parse_rst_stream(frame.payload())?;

        // stream was never opened
        if frame.stream_id() > self.last_stream_id {
            return Err(AssembleError::InvalidStreamId);
        }

        self.streams.remove(&frame.stream_id());
        self.reset.insert(frame.stream_id());
        Ok(None)
    }

    fn dispatch(&mut self, frame: &Frame) -> Result<Option<Assembled>, AssembleError> {
        if let Some(pending) = self.pending.take() {
            self.on_continuation(frame, pending)
        } else {
            match frame.frame_type() {
                FrameType::Data => self.on_data(frame),
                FrameType::Headers => self.on_header(frame),
                FrameType::RstStream => self.on_rst_stream(frame),
                FrameType::Continuation => Err(AssembleError::UnexpectedContinuation),
                _ => Ok(None),
            }
        }
    }

    pub fn on_frame(&mut self, frame: &Frame) -> Result<Option<Assembled>, AssembleError> {
        let result = self.dispatch(frame);

        if let Err(error) = &result
            && !error.is_connection_error()
        {
            self.reset.insert(frame.stream_id());
        }

        result
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
    fn feed(assembler: &mut Assembler, frames: Vec<Frame>) -> Vec<Assembled> {
        frames
            .into_iter()
            .filter_map(|frame| assembler.on_frame(&frame).unwrap())
            .collect()
    }

    // Feeds every frame but the last without error, returns the result of the last one.
    fn last(
        assembler: &mut Assembler,
        mut frames: Vec<Frame>,
    ) -> Result<Option<Assembled>, AssembleError> {
        let last = frames.pop().unwrap();
        feed(assembler, frames);
        assembler.on_frame(&last)
    }

    fn single(frames: Vec<Frame>) -> Assembled {
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
        let assembled = single(vec![headers(1, end_all(), GET)]);

        assert_eq!(assembled.stream_id, 1);
        assert!(matches!(assembled.request.method(), Method::Get));
        assert_eq!(assembled.request.path(), "/echo/abc");
        assert_eq!(assembled.request.body(), b"");
    }

    // RFC 7541 Appendix C.4.1, Huffman-encoded, wrapped in a HEADERS frame.
    #[test]
    fn rfc_7541_c_4_1_request() {
        let block = [
            0x82, 0x86, 0x84, 0x41, 0x8c, 0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab,
            0x90, 0xf4, 0xff,
        ];

        let assembled = single(vec![raw_headers(1, end_all(), block)]);

        assert!(matches!(assembled.request.method(), Method::Get));
        assert_eq!(assembled.request.path(), "/");
        assert_eq!(assembled.request.header("host"), Some("www.example.com"));
    }

    // RFC 9113 §8.3.1: handlers read `Host`; HTTP/2 carries it in `:authority`.
    #[test]
    fn authority_becomes_host() {
        let assembled = single(vec![headers(1, end_all(), GET)]);

        assert_eq!(assembled.request.header("Host"), Some("localhost:4221"));
    }

    #[test]
    fn pseudo_headers_are_not_regular_headers() {
        let fields = with(&[("user-agent", "curl/8.7.1")]);

        let assembled = single(vec![headers(1, end_all(), &fields)]);

        assert_eq!(assembled.request.header(":path"), None);
        assert_eq!(assembled.request.header(":method"), None);
        assert_eq!(assembled.request.header("User-Agent"), Some("curl/8.7.1"));
    }

    // Without END_STREAM, a body may follow: the request is not complete yet.
    #[test]
    fn headers_without_end_stream_wait_for_data() {
        let mut assembler = Assembler::new();

        let result = assembler.on_frame(&headers(1, Flags::END_HEADERS, POST));

        assert!(matches!(result, Ok(None)));
    }

    // Frames that belong to the connection are not the assembler's business.
    #[test]
    fn connection_frames_are_ignored() {
        let mut assembler = Assembler::new();

        assert!(matches!(assembler.on_frame(&ping()), Ok(None)));
    }

    // --- Body (RFC 9113 §6.1) ---

    #[test]
    fn body_spread_over_two_data_frames() {
        let mut assembler = Assembler::new();

        assert!(matches!(
            assembler.on_frame(&headers(1, Flags::END_HEADERS, POST)),
            Ok(None)
        ));
        assert!(matches!(
            assembler.on_frame(&data(1, Flags::empty(), b"hel")),
            Ok(None)
        ));
        let assembled = assembler
            .on_frame(&data(1, Flags::END_STREAM, b"lo"))
            .unwrap()
            .unwrap();

        assert_eq!(assembled.stream_id, 1);
        assert!(matches!(assembled.request.method(), Method::Post));
        assert_eq!(assembled.request.body(), b"hello");
    }

    // curl often ends the body with an empty DATA frame carrying END_STREAM.
    #[test]
    fn empty_data_frame_can_end_the_stream() {
        let assembled = single(vec![
            headers(1, Flags::END_HEADERS, POST),
            data(1, Flags::empty(), b"abc"),
            data(1, Flags::END_STREAM, b""),
        ]);

        assert_eq!(assembled.request.body(), b"abc");
    }

    // A body is raw bytes: nothing forces it to be UTF-8.
    #[test]
    fn body_is_binary() {
        let assembled = single(vec![
            headers(1, Flags::END_HEADERS, POST),
            data(1, Flags::END_STREAM, &[0xff, 0x00, 0xfe]),
        ]);

        assert_eq!(assembled.request.body(), &[0xff, 0x00, 0xfe]);
    }

    // PADDED: first byte = pad length, then data, then that many padding bytes.
    #[test]
    fn padded_data_drops_the_padding() {
        let assembled = single(vec![
            headers(1, Flags::END_HEADERS, POST),
            data(1, Flags::PADDED | Flags::END_STREAM, &[2, b'h', b'i', 0, 0]),
        ]);

        assert_eq!(assembled.request.body(), b"hi");
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

        let summary: Vec<(u32, &[u8])> = requests
            .iter()
            .map(|assembled| (assembled.stream_id, assembled.request.body()))
            .collect();
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

        let assembled = single(vec![
            headers(1, Flags::END_HEADERS, &fields),
            data(1, Flags::END_STREAM, b"hi"),
        ]);

        assert_eq!(assembled.request.body(), b"hi");
    }

    // --- Header block: padding, priority, CONTINUATION (RFC 9113 §6.2, §6.10) ---

    #[test]
    fn header_block_split_over_continuation() {
        let block = encode(GET);
        let (first, rest) = block.split_at(5);

        let mut assembler = Assembler::new();
        assert!(matches!(
            assembler.on_frame(&raw_headers(1, Flags::END_STREAM, first)),
            Ok(None)
        ));
        let assembled = assembler
            .on_frame(&continuation(1, Flags::END_HEADERS, rest))
            .unwrap()
            .unwrap();

        assert_eq!(assembled.stream_id, 1);
        assert_eq!(assembled.request.path(), "/echo/abc");
    }

    // Splits can fall anywhere, even inside a string: decode only once the block is whole.
    #[test]
    fn header_block_split_over_two_continuations() {
        let block = encode(GET);

        let assembled = single(vec![
            raw_headers(1, Flags::END_STREAM, &block[..3]),
            continuation(1, Flags::empty(), &block[3..10]),
            continuation(1, Flags::END_HEADERS, &block[10..]),
        ]);

        assert_eq!(assembled.request.path(), "/echo/abc");
    }

    #[test]
    fn padded_headers_drop_the_padding() {
        let mut payload = vec![3];
        payload.extend(encode(GET));
        payload.extend([0, 0, 0]);

        let assembled = single(vec![raw_headers(1, end_all() | Flags::PADDED, payload)]);

        assert_eq!(assembled.request.path(), "/echo/abc");
    }

    // PRIORITY: 5 bytes (E + stream dependency on 31 bits, weight) to skip, deprecated.
    #[test]
    fn priority_fields_are_skipped() {
        // E = 1, dependency = 0, weight = 15
        let mut payload = vec![0x80, 0, 0, 0, 15];
        payload.extend(encode(GET));

        let assembled = single(vec![raw_headers(1, end_all() | Flags::PRIORITY, payload)]);

        assert_eq!(assembled.request.path(), "/echo/abc");
    }

    // Both flags: pad length, then priority, then the block, then the padding.
    #[test]
    fn padded_and_priority_together() {
        let mut payload = vec![2, 0, 0, 0, 0, 15];
        payload.extend(encode(GET));
        payload.extend([0, 0]);

        let flags = end_all() | Flags::PADDED | Flags::PRIORITY;
        let assembled = single(vec![raw_headers(1, flags, payload)]);

        assert_eq!(assembled.request.path(), "/echo/abc");
    }

    // RFC 9113 §6.2: padding as long as (or longer than) the rest is a PROTOCOL_ERROR.
    #[test]
    fn padding_longer_than_the_payload_is_an_error() {
        let payload = vec![10, 0x82, 0x86];

        let result = Assembler::new().on_frame(&raw_headers(1, end_all() | Flags::PADDED, payload));

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
        let result = Assembler::new().on_frame(&raw_headers(1, end_all(), [0xff]));

        assert!(matches!(result, Err(AssembleError::Hpack(_))));
    }

    // --- Stream ids (RFC 9113 §5.1.1) ---

    #[test]
    fn even_stream_id_is_an_error() {
        let result = Assembler::new().on_frame(&headers(2, end_all(), GET));

        assert!(matches!(result, Err(AssembleError::InvalidStreamId)));
    }

    #[test]
    fn stream_id_zero_is_an_error() {
        let result = Assembler::new().on_frame(&headers(0, end_all(), GET));

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

        let ids: Vec<u32> = requests
            .iter()
            .map(|assembled| assembled.stream_id)
            .collect();
        assert_eq!(ids, vec![1, 7]);
    }

    // --- Stream states (RFC 9113 §5.1) ---

    // DATA on a stream that was never opened (idle): PROTOCOL_ERROR.
    #[test]
    fn data_on_an_idle_stream_is_an_error() {
        let result = Assembler::new().on_frame(&data(1, Flags::END_STREAM, b"hi"));

        assert!(matches!(result, Err(AssembleError::InvalidStreamId)));
    }

    #[test]
    fn data_on_stream_zero_is_an_error() {
        let result = Assembler::new().on_frame(&data(0, Flags::END_STREAM, b"hi"));

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

    // --- Reset streams (RFC 9113 §5.1, step 8) ---
    //
    // After a stream error the server sends RST_STREAM, but the client may have
    // sent more frames on that stream before receiving it: they MUST be ignored.

    #[test]
    fn data_after_a_malformed_request_is_ignored() {
        let mut assembler = Assembler::new();

        let rejected = assembler.on_frame(&headers(1, Flags::END_HEADERS, &without(":path")));
        let late = assembler.on_frame(&data(1, Flags::END_STREAM, b"hi"));

        assert!(matches!(rejected, Err(AssembleError::MalformedRequest)));
        assert!(matches!(late, Ok(None)));
    }

    #[test]
    fn data_after_a_content_length_mismatch_is_ignored() {
        let fields = [
            (":method", "POST"),
            (":scheme", "http"),
            (":path", "/files/a"),
            ("content-length", "5"),
        ];
        let mut assembler = Assembler::new();
        feed(
            &mut assembler,
            vec![headers(1, Flags::END_HEADERS, &fields)],
        );

        let rejected = assembler.on_frame(&data(1, Flags::END_STREAM, b"hi"));
        let late = assembler.on_frame(&data(1, Flags::END_STREAM, b"more"));

        assert!(matches!(rejected, Err(AssembleError::MalformedRequest)));
        assert!(matches!(late, Ok(None)));
    }

    // The first late DATA is STREAM_CLOSED; the following ones are ignored.
    #[test]
    fn data_after_stream_closed_is_ignored() {
        let mut assembler = Assembler::new();
        feed(&mut assembler, vec![headers(1, end_all(), GET)]);

        let first = assembler.on_frame(&data(1, Flags::END_STREAM, b"late"));
        let second = assembler.on_frame(&data(1, Flags::END_STREAM, b"later"));

        assert!(matches!(first, Err(AssembleError::StreamClosed)));
        assert!(matches!(second, Ok(None)));
    }

    // Resetting stream 1 has no effect on stream 3.
    #[test]
    fn a_reset_stream_does_not_affect_the_others() {
        let mut assembler = Assembler::new();
        let rejected = assembler.on_frame(&headers(1, Flags::END_HEADERS, &without(":path")));

        let assembled = assembler
            .on_frame(&headers(3, end_all(), GET))
            .unwrap()
            .unwrap();
        let late = assembler.on_frame(&data(1, Flags::END_STREAM, b"hi"));

        assert!(matches!(rejected, Err(AssembleError::MalformedRequest)));
        assert_eq!(assembled.stream_id, 3);
        assert!(matches!(late, Ok(None)));
    }

    // Only stream errors are remembered: DATA on a never-opened stream stays a
    // connection error, even when repeated.
    #[test]
    fn data_on_an_idle_stream_is_never_ignored() {
        let mut assembler = Assembler::new();

        let first = assembler.on_frame(&data(5, Flags::END_STREAM, b"hi"));
        let second = assembler.on_frame(&data(5, Flags::END_STREAM, b"hi"));

        assert!(matches!(first, Err(AssembleError::InvalidStreamId)));
        assert!(matches!(second, Err(AssembleError::InvalidStreamId)));
    }

    // --- Malformed requests (RFC 9113 §8.1.1, §8.2, §8.3.1): stream errors ---

    fn assert_malformed(fields: &[(&str, &str)]) {
        let result = Assembler::new().on_frame(&headers(1, end_all(), fields));

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
        let assembled = single(vec![headers(1, end_all(), &without(":authority"))]);

        assert_eq!(assembled.request.path(), "/echo/abc");
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

        let assembled = single(vec![headers(1, end_all(), &with(&[("te", "trailers")]))]);
        assert_eq!(assembled.request.path(), "/echo/abc");
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
        let first = assembler.on_frame(&raw_headers(1, end_all(), stream_1));
        let assembled = assembler
            .on_frame(&raw_headers(3, end_all(), stream_3))
            .unwrap()
            .unwrap();

        assert!(matches!(first, Err(AssembleError::MalformedRequest)));
        assert_eq!(assembled.stream_id, 3);
        assert_eq!(assembled.request.header("host"), Some("www.example.com"));
    }

    // --- Field validity (RFC 9113 §8.2.1) ---
    //
    // `encode` only takes &str: these blocks are built by hand to reach bytes
    // that are not UTF-8, or names HPACK allows but HTTP/2 does not.

    // RFC 7541 §6.2.2, literal without indexing, new name. Lengths < 127 only.
    fn literal(name: &[u8], value: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x00, name.len() as u8];
        bytes.extend_from_slice(name);
        bytes.push(value.len() as u8);
        bytes.extend_from_slice(value);
        bytes
    }

    // A valid GET followed by one raw field.
    fn get_with_raw(name: &[u8], value: &[u8]) -> Result<Option<Assembled>, AssembleError> {
        let mut block = encode(GET);
        block.extend(literal(name, value));
        Assembler::new().on_frame(&raw_headers(1, end_all(), block))
    }

    fn assert_raw_malformed(name: &[u8], value: &[u8]) {
        assert!(
            matches!(
                get_with_raw(name, value),
                Err(AssembleError::MalformedRequest)
            ),
            "expected MalformedRequest for name {name:?}, value {value:?}"
        );
    }

    fn assert_raw_accepted(name: &[u8], value: &[u8]) -> Request {
        match get_with_raw(name, value) {
            Ok(Some(assembled)) => assembled.request,
            _ => panic!("expected a request for name {name:?}, value {value:?}"),
        }
    }

    // HPACK can carry a zero-length name; HTTP/2 cannot use it.
    // Regression: `split_at(1)` on an empty name panicked.
    #[test]
    fn empty_header_name_is_malformed() {
        assert_raw_malformed(b"", b"x");
    }

    // Regression: `split_at(1)` panicked when byte 1 is inside a multi-byte character.
    #[test]
    fn non_ascii_header_name_is_malformed() {
        assert_raw_malformed("éa".as_bytes(), b"x");
        assert_raw_malformed(&[0xff, b'a'], b"x");
    }

    // A lone colon is a pseudo-header with an empty, hence unknown, name.
    #[test]
    fn lone_colon_is_malformed() {
        assert_raw_malformed(b":", b"x");
    }

    // Forbidden in names: 0x00-0x20 (controls and space), 0x7f (DEL), 0x80-0xff.
    #[test]
    fn control_space_and_del_in_name_are_malformed() {
        for name in [&b"x\0y"[..], b"x\ty", b"x\ry", b"x\ny", b"x y", b"x\x7fy"] {
            assert_raw_malformed(name, b"v");
        }
    }

    // Outside pseudo-headers, a name must not contain a colon at all.
    #[test]
    fn colon_inside_a_regular_name_is_malformed() {
        assert_raw_malformed(b"x:y", b"v");
    }

    // Header injection (h2 → h1 downgrade): CR, LF and NUL are forbidden in values.
    #[test]
    fn cr_lf_or_nul_in_value_is_malformed() {
        for value in [&b"a\rb"[..], b"a\nb", b"a\r\nevil: 1", b"a\0b"] {
            assert_raw_malformed(b"x-test", value);
        }
    }

    #[test]
    fn value_starting_or_ending_with_whitespace_is_malformed() {
        for value in [&b" a"[..], b"a ", b"\ta", b"a\t"] {
            assert_raw_malformed(b"x-test", value);
        }
    }

    // The value rules apply to pseudo-headers too.
    #[test]
    fn crlf_in_path_is_malformed() {
        assert_malformed(&[
            (":method", "GET"),
            (":scheme", "http"),
            (":path", "/a\r\nx-evil: 1"),
        ]);
    }

    // Inside a value, spaces and tabs are fine, and so is an empty value.
    #[test]
    fn inner_whitespace_and_empty_values_are_accepted() {
        let request = assert_raw_accepted(b"x-test", b"a b\tc");
        assert_eq!(request.header("x-test"), Some("a b\tc"));

        let request = assert_raw_accepted(b"accept", b"");
        assert_eq!(request.header("accept"), Some(""));
    }

    // Token characters other than letters and digits are allowed in names.
    #[test]
    fn punctuation_in_name_is_accepted() {
        let request = assert_raw_accepted(b"x-a_b.c!#$%&'*+^`|~", b"v");
        assert_eq!(request.header("x-a_b.c!#$%&'*+^`|~"), Some("v"));
    }

    // Values may hold bytes >= 0x80: UTF-8 text stays readable.
    #[test]
    fn non_ascii_value_is_accepted() {
        let request = assert_raw_accepted(b"x-test", "café".as_bytes());
        assert_eq!(request.header("x-test"), Some("café"));
    }
}
