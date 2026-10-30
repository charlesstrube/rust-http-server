use std::{
    io::{Error, ErrorKind, Read},
    ops::BitOr,
};

pub(crate) const DEFAULT_MAX_FRAME_SIZE: u32 = 16_384;
const MAX_FRAME_LENGTH: usize = 0xFF_FFFF;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameType {
    Data,
    Headers,
    Priority,
    RstStream,
    Settings,
    PushPromise,
    Ping,
    GoAway,
    WindowUpdate,
    Continuation,
    Unknown(u8),
}

impl From<u8> for FrameType {
    fn from(value: u8) -> Self {
        match value {
            0x0 => FrameType::Data,
            0x1 => FrameType::Headers,
            0x2 => FrameType::Priority,
            0x3 => FrameType::RstStream,
            0x4 => FrameType::Settings,
            0x5 => FrameType::PushPromise,
            0x6 => FrameType::Ping,
            0x7 => FrameType::GoAway,
            0x8 => FrameType::WindowUpdate,
            0x9 => FrameType::Continuation,
            value => FrameType::Unknown(value),
        }
    }
}

impl From<FrameType> for u8 {
    fn from(value: FrameType) -> Self {
        match value {
            FrameType::Data => 0x0,
            FrameType::Headers => 0x1,
            FrameType::Priority => 0x2,
            FrameType::RstStream => 0x3,
            FrameType::Settings => 0x4,
            FrameType::PushPromise => 0x5,
            FrameType::Ping => 0x6,
            FrameType::GoAway => 0x7,
            FrameType::WindowUpdate => 0x8,
            FrameType::Continuation => 0x9,
            FrameType::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flags(u8);

impl Flags {
    pub const ACK: Flags = Flags(0x01); // 0000 0001
    pub const PADDED: Flags = Flags(0x08); // 0000 1000
    pub const END_STREAM: Flags = Flags(0x01); // 0000 0001
    pub const END_HEADERS: Flags = Flags(0x04); // 0000 0100
    pub const PRIORITY: Flags = Flags(0x20); // 0010 0000

    pub fn contains(&self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn empty() -> Self {
        Self(0)
    }
}

impl BitOr<Flags> for Flags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FrameError {
    #[error("Truncated")]
    Truncated,
    #[error("Too Large")]
    TooLarge { length: u32, max: u32 },
    #[error("I/O error")]
    Io(String),
}

#[derive(Debug, PartialEq)]
struct FrameHeader {
    length: u32,
    frame_type: FrameType,
    flags: Flags,
    stream_id: u32,
}

impl From<Error> for FrameError {
    fn from(err: Error) -> Self {
        match err.kind() {
            // if it's truncated before the end of buffer
            ErrorKind::UnexpectedEof => FrameError::Truncated,
            // maps the rest of errors to Io
            _ => FrameError::Io(err.to_string()),
        }
    }
}

impl FrameHeader {
    pub fn parse(df: &[u8; 9]) -> Self {
        let l_0 = u32::from(df[0]) << 16;
        let l_1 = u32::from(df[1]) << 8;
        let l_2 = u32::from(df[2]);
        let length = l_0 | l_1 | l_2;
        let frame_type = FrameType::from(df[3]);
        let flags = Flags(df[4]);

        let stream_id = u32::from_be_bytes([df[5], df[6], df[7], df[8]]) & 0x7FFF_FFFF;

        Self {
            length,
            frame_type,
            flags,
            stream_id,
        }
    }

    pub fn to_bytes(&self) -> [u8; 9] {
        let mut result: [u8; 9] = [0; 9];

        result[0] = ((self.length >> 16) & 0xff) as u8;
        result[1] = ((self.length >> 8) & 0xff) as u8;
        result[2] = self.length as u8;
        result[3] = u8::from(self.frame_type);
        result[4] = self.flags.0;

        // RFC 9113 §4.1: the reserved bit must remain unset when sending
        // value  : 1000 0000 0000 0000 0000 0000 0000 0001   (0x8000_0001)
        // mask   : 0111 1111 1111 1111 1111 1111 1111 1111   (0x7FFF_FFFF)
        // ──────────────────────────────────────────────── &
        // result : 0000 0000 0000 0000 0000 0000 0000 0001   (0x0000_0001 = 1)
        //          ↑
        //          the R bit was set → cleared; every other bit is kept
        let masked = self.stream_id & 0x7FFF_FFFF;

        let stream_id = masked.to_be_bytes();

        result[5..].copy_from_slice(&stream_id);
        result
    }
}

#[derive(Debug, PartialEq)]
pub struct Frame {
    frame_type: FrameType,
    flags: Flags,
    stream_id: u32,
    payload: Vec<u8>,
}

impl Frame {
    pub fn new(
        frame_type: FrameType,
        flags: Flags,
        stream_id: u32,
        payload: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            frame_type,
            flags,
            stream_id,
            payload: payload.into(),
        }
    }

    pub fn frame_type(&self) -> FrameType {
        self.frame_type
    }

    pub fn flags(&self) -> Flags {
        self.flags
    }

    pub fn stream_id(&self) -> &u32 {
        &self.stream_id
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        debug_assert!(self.payload.len() <= MAX_FRAME_LENGTH);
        let frame_header = FrameHeader {
            length: self.payload.len() as u32,
            frame_type: self.frame_type,
            flags: self.flags,
            stream_id: self.stream_id,
        };

        let mut response = frame_header.to_bytes().to_vec();
        response.extend_from_slice(&self.payload);

        response
    }
}

fn read_frame_payload(
    read: &mut impl Read,
    frame_header: FrameHeader,
) -> Result<Frame, FrameError> {
    let mut payload: Vec<u8> = vec![0u8; frame_header.length as usize];

    // read and remap errors to FrameError with from<Error>
    read.read_exact(&mut payload)?;

    let frame = Frame::new(
        frame_header.frame_type,
        frame_header.flags,
        frame_header.stream_id,
        payload,
    );

    Ok(frame)
}

fn read_header(read: &mut impl Read) -> Result<Option<FrameHeader>, FrameError> {
    let mut buffer = [0u8; 9];
    let mut filled = 0;

    while filled < buffer.len() {
        // match on the bytes read
        match read.read(&mut buffer[filled..]) {
            // if source (read) has 0 bytes and buffer has 0 bytes filled, it's clean
            Ok(0) if filled == 0 => return Ok(None),
            // source doesn't have any butes left but we didn't reach the end of the buffer
            Ok(0) => return Err(FrameError::Truncated),
            // add
            Ok(value) => filled += value,
            // restart the loop
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            // err
            Err(err) => return Err(err.into()),
        }
    }

    Ok(Some(FrameHeader::parse(&buffer)))
}

pub fn read_frame(read: &mut impl Read, max: u32) -> Result<Option<Frame>, FrameError> {
    let Some(frame_header) = read_header(read)? else {
        return Ok(None);
    };

    if frame_header.length > max {
        return Err(FrameError::TooLarge {
            length: frame_header.length,
            max,
        });
    }

    Ok(Some(read_frame_payload(read, frame_header)?))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::{BufReader, Cursor};

    const MAX: u32 = DEFAULT_MAX_FRAME_SIZE;

    // --- FrameType (RFC 9113 §6) ---

    #[test]
    fn known_frame_types_have_their_code() {
        let table = [
            (0x0, FrameType::Data),
            (0x1, FrameType::Headers),
            (0x2, FrameType::Priority),
            (0x3, FrameType::RstStream),
            (0x4, FrameType::Settings),
            (0x5, FrameType::PushPromise),
            (0x6, FrameType::Ping),
            (0x7, FrameType::GoAway),
            (0x8, FrameType::WindowUpdate),
            (0x9, FrameType::Continuation),
        ];

        for (code, frame_type) in table {
            assert_eq!(FrameType::from(code), frame_type, "for {code:#x}");
            assert_eq!(u8::from(frame_type), code, "for {frame_type:?}");
        }
    }

    // RFC 9113 §4.1: unknown types must be ignored, so they cannot be a parse error.
    #[test]
    fn unknown_frame_type_keeps_its_code() {
        for code in [0x0a, 0x42, 0xff] {
            assert_eq!(FrameType::from(code), FrameType::Unknown(code));
            assert_eq!(u8::from(FrameType::Unknown(code)), code);
        }
    }

    // --- Flags ---

    #[test]
    fn empty_flags_contain_nothing() {
        let flags = Flags::empty();

        assert!(!flags.contains(Flags::END_STREAM));
        assert!(!flags.contains(Flags::END_HEADERS));
    }

    #[test]
    fn combined_flags_contain_each_part() {
        let flags = Flags::END_STREAM | Flags::END_HEADERS;

        assert!(flags.contains(Flags::END_STREAM));
        assert!(flags.contains(Flags::END_HEADERS));
        assert!(!flags.contains(Flags::PADDED));
        assert!(!flags.contains(Flags::PRIORITY));
    }

    // RFC 9113 §6: flag values are per frame type; ACK (SETTINGS, PING) reuses bit 0x1.
    #[test]
    fn ack_shares_the_end_stream_bit() {
        assert_eq!(Flags::ACK, Flags::END_STREAM);
    }

    // --- FrameHeader: the fixed 9 bytes (RFC 9113 §4.1) ---

    #[test]
    fn parses_empty_settings_header() {
        let header = FrameHeader::parse(&[0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00]);

        assert_eq!(
            header,
            FrameHeader {
                length: 0,
                frame_type: FrameType::Settings,
                flags: Flags::empty(),
                stream_id: 0,
            }
        );
    }

    // The length is a 24-bit big-endian integer: there is no u24, it must be assembled by hand.
    #[test]
    fn length_is_24_bit_big_endian() {
        let header = FrameHeader::parse(&[0x01, 0x02, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        assert_eq!(header.length, 0x01_02_03);
    }

    // Parsing does not validate the size: that is `read_frame`'s job, against the negotiated max.
    #[test]
    fn parses_maximum_24_bit_length() {
        let header = FrameHeader::parse(&[0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        assert_eq!(header.length, 16_777_215);
    }

    #[test]
    fn stream_id_is_31_bit_big_endian() {
        let header = FrameHeader::parse(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01]);

        assert_eq!(header.stream_id, 257);
    }

    // RFC 9113 §4.1: the reserved bit "MUST be ignored when receiving".
    #[test]
    fn reserved_bit_is_ignored_when_parsing() {
        let header = FrameHeader::parse(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x01]);
        assert_eq!(header.stream_id, 1);

        let header = FrameHeader::parse(&[0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(header.stream_id, 0x7fff_ffff);
    }

    #[test]
    fn parses_flags_and_type() {
        // HEADERS on stream 3 with END_STREAM | END_HEADERS (0x01 | 0x04)
        let header = FrameHeader::parse(&[0x00, 0x00, 0x00, 0x01, 0x05, 0x00, 0x00, 0x00, 0x03]);

        assert_eq!(header.frame_type, FrameType::Headers);
        assert!(header.flags.contains(Flags::END_STREAM));
        assert!(header.flags.contains(Flags::END_HEADERS));
        assert!(!header.flags.contains(Flags::PADDED));
        assert_eq!(header.stream_id, 3);
    }

    #[test]
    fn parses_unknown_type() {
        let header = FrameHeader::parse(&[0x00, 0x00, 0x00, 0xbe, 0x00, 0x00, 0x00, 0x00, 0x00]);

        assert_eq!(header.frame_type, FrameType::Unknown(0xbe));
    }

    #[test]
    fn header_round_trips() {
        let header = FrameHeader {
            length: 0x00_12_34,
            frame_type: FrameType::Data,
            flags: Flags::END_STREAM | Flags::PADDED,
            stream_id: 0x0102_0304,
        };

        assert_eq!(
            header.to_bytes(),
            [0x00, 0x12, 0x34, 0x00, 0x09, 0x01, 0x02, 0x03, 0x04]
        );
        assert_eq!(FrameHeader::parse(&header.to_bytes()), header);
    }

    // RFC 9113 §4.1: the reserved bit "MUST remain unset (0x00) when sending".
    #[test]
    fn reserved_bit_is_cleared_when_serializing() {
        let header = FrameHeader {
            length: 0,
            frame_type: FrameType::Ping,
            flags: Flags::empty(),
            stream_id: 0xffff_ffff,
        };

        assert_eq!(&header.to_bytes()[5..], [0x7f, 0xff, 0xff, 0xff]);
    }

    // --- read_frame: header + payload from a byte stream ---

    #[test]
    fn reads_a_ping_frame() {
        let mut reader = Cursor::new(vec![
            0x00, 0x00, 0x08, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, // PING, 8 bytes, stream 0
            1, 2, 3, 4, 5, 6, 7, 8,
        ]);

        let frame = read_frame(&mut reader, MAX).unwrap().unwrap();

        assert_eq!(
            frame,
            Frame::new(FrameType::Ping, Flags::empty(), 0, [1, 2, 3, 4, 5, 6, 7, 8])
        );
    }

    #[test]
    fn reads_a_data_frame() {
        let mut reader = Cursor::new(vec![
            0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, // DATA, END_STREAM, stream 1
            b'a', b'b', b'c',
        ]);

        let frame = read_frame(&mut reader, MAX).unwrap().unwrap();

        assert_eq!(
            frame,
            Frame::new(FrameType::Data, Flags::END_STREAM, 1, *b"abc")
        );
    }

    #[test]
    fn reads_a_frame_without_payload() {
        // SETTINGS ACK
        let mut reader = Cursor::new(vec![0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00]);

        let frame = read_frame(&mut reader, MAX).unwrap().unwrap();

        assert_eq!(frame, Frame::new(FrameType::Settings, Flags::ACK, 0, []));
    }

    // Like HTTP/1.1 pipelining: a frame must not consume a byte of the next one.
    #[test]
    fn reads_consecutive_frames_from_the_same_reader() {
        let mut reader = Cursor::new(vec![
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, b'h', b'i', // DATA stream 1
            0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x03,
            b'!', // DATA stream 3, END_STREAM
        ]);

        let first = read_frame(&mut reader, MAX).unwrap().unwrap();
        let second = read_frame(&mut reader, MAX).unwrap().unwrap();

        assert_eq!(
            first,
            Frame::new(FrameType::Data, Flags::empty(), 1, *b"hi")
        );
        assert_eq!(
            second,
            Frame::new(FrameType::Data, Flags::END_STREAM, 3, *b"!")
        );
        assert!(matches!(read_frame(&mut reader, MAX), Ok(None)));
    }

    // Worst-case TCP fragmentation: a single `read` call is never enough.
    #[test]
    fn reads_a_frame_received_byte_by_byte() {
        let bytes = vec![
            0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, b'a', b'b', b'c',
        ];
        let mut reader = BufReader::with_capacity(1, Cursor::new(bytes));

        let frame = read_frame(&mut reader, MAX).unwrap().unwrap();

        assert_eq!(
            frame,
            Frame::new(FrameType::Data, Flags::END_STREAM, 1, *b"abc")
        );
    }

    // The codec reads unknown types; the connection layer is the one that ignores them.
    #[test]
    fn reads_an_unknown_frame_type() {
        let mut reader = Cursor::new(vec![
            0x00, 0x00, 0x01, 0xbe, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2a,
        ]);

        let frame = read_frame(&mut reader, MAX).unwrap().unwrap();

        assert_eq!(
            frame,
            Frame::new(FrameType::Unknown(0xbe), Flags::empty(), 0, [0x2a])
        );
    }

    // EOF before any byte: the client closed the connection between two frames.
    #[test]
    fn eof_before_any_byte_is_none() {
        let mut reader = Cursor::new(vec![]);

        assert!(matches!(read_frame(&mut reader, MAX), Ok(None)));
    }

    #[test]
    fn eof_in_the_middle_of_the_header_is_truncated() {
        let mut reader = Cursor::new(vec![0x00, 0x00, 0x08, 0x06, 0x00]);

        assert_eq!(read_frame(&mut reader, MAX), Err(FrameError::Truncated));
    }

    #[test]
    fn eof_in_the_middle_of_the_payload_is_truncated() {
        let mut reader = Cursor::new(vec![
            0x00, 0x00, 0x08, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, // announces 8 bytes
            1, 2, 3,
        ]);

        assert_eq!(read_frame(&mut reader, MAX), Err(FrameError::Truncated));
    }

    // Once the header is read, the frame has started: EOF is never a clean close here.
    #[test]
    fn eof_right_after_the_header_is_truncated() {
        let mut reader = Cursor::new(vec![
            0x00, 0x00, 0x08, 0x06, 0x00, 0x00, 0x00, 0x00,
            0x00, // announces 8 bytes, none follow
        ]);

        assert_eq!(read_frame(&mut reader, MAX), Err(FrameError::Truncated));
    }

    // RFC 9113 §4.2: larger than SETTINGS_MAX_FRAME_SIZE is a FRAME_SIZE_ERROR.
    // No payload follows: the size must be checked before allocating or reading it.
    #[test]
    fn frame_larger_than_max_is_rejected_before_reading_the_payload() {
        // 16_385 = 0x00_40_01
        let mut reader = Cursor::new(vec![0x00, 0x40, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);

        assert_eq!(
            read_frame(&mut reader, MAX),
            Err(FrameError::TooLarge {
                length: 16_385,
                max: 16_384
            })
        );
    }

    #[test]
    fn frame_of_exactly_max_size_is_accepted() {
        let mut bytes = vec![0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        bytes.extend(vec![0u8; 16_384]);
        let mut reader = Cursor::new(bytes);

        let frame = read_frame(&mut reader, MAX).unwrap().unwrap();

        assert_eq!(frame.payload().len(), 16_384);
    }

    // The limit is the one we announced in our SETTINGS, not a constant.
    #[test]
    fn negotiated_max_frame_size_is_respected() {
        let mut bytes = vec![0x00, 0x40, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        bytes.extend(vec![0u8; 16_385]);
        let mut reader = Cursor::new(bytes);

        let frame = read_frame(&mut reader, 20_000).unwrap().unwrap();

        assert_eq!(frame.payload().len(), 16_385);
    }

    // --- Frame::to_bytes ---

    #[test]
    fn serializes_empty_settings() {
        let frame = Frame::new(FrameType::Settings, Flags::empty(), 0, []);

        assert_eq!(
            frame.to_bytes(),
            [0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn serializes_settings_ack() {
        let frame = Frame::new(FrameType::Settings, Flags::ACK, 0, []);

        assert_eq!(
            frame.to_bytes(),
            [0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00]
        );
    }

    // The length field is derived from the payload, it cannot disagree with it.
    #[test]
    fn length_comes_from_the_payload() {
        let frame = Frame::new(FrameType::Data, Flags::END_STREAM, 1, vec![b'x'; 300]);

        let bytes = frame.to_bytes();

        // 300 = 0x00_01_2c
        assert_eq!(
            bytes[..9],
            [0x00, 0x01, 0x2c, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]
        );
        assert_eq!(bytes.len(), 9 + 300);
        assert!(bytes[9..].iter().all(|&byte| byte == b'x'));
    }

    #[test]
    fn frame_round_trips() {
        let frame = Frame::new(
            FrameType::Headers,
            Flags::END_HEADERS | Flags::END_STREAM,
            5,
            *b"hpack block",
        );

        let mut reader = Cursor::new(frame.to_bytes());

        assert_eq!(read_frame(&mut reader, MAX).unwrap().unwrap(), frame);
    }
}
