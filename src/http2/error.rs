// Error signalling (RFC 9113 §5.4, §6.4, §6.8, §7).
//
// - `ErrorCode`: the 32-bit codes shared by RST_STREAM and GOAWAY. Unknown
//   codes keep their value and carry no special meaning (§7).
// - RST_STREAM (§6.4): stream error, cancels one stream, the connection goes on.
// - GOAWAY (§6.8): connection error or shutdown, tells the peer the last
//   stream that was (or may be) processed; streams above it are safe to retry.
// - Conversions from the module errors (`SettingsError`, `PayloadError`) to
//   the code sent to the peer.

use crate::http2::{
    frame::{Flags, Frame, FrameType},
    settings::SettingsError,
};

const RESERVED_BIT_MASK: u32 = 0x7FFF_FFFF;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum PayloadError {
    #[error("Length")]
    Length,
}

impl From<PayloadError> for ErrorCode {
    fn from(_: PayloadError) -> Self {
        Self::FrameSizeError
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ErrorCode {
    NoError,
    ProtocolError,
    InternalError,
    FlowControlError,
    SettingsTimeout,
    StreamClosed,
    FrameSizeError,
    RefusedStream,
    Cancel,
    CompressionError,
    ConnectError,
    EnhanceYourCalm,
    InadequateSecurity,
    Http11Required,
    Unknown(u32),
}

#[derive(Debug, PartialEq)]
pub struct GoAway {
    last_stream_id: u32,
    error_code: ErrorCode,
    debug_data: Vec<u8>,
}

impl GoAway {
    pub fn parse(payload: &[u8]) -> Result<GoAway, PayloadError> {
        let Some((id_bytes, rest)) = payload.split_first_chunk::<4>() else {
            return Err(PayloadError::Length);
        };

        let Some((error_bytes, debug_data)) = rest.split_first_chunk::<4>() else {
            return Err(PayloadError::Length);
        };

        let id = u32::from_be_bytes(*id_bytes) & RESERVED_BIT_MASK;
        let error_code = ErrorCode::from(u32::from_be_bytes(*error_bytes));

        Ok(GoAway {
            last_stream_id: id,
            error_code,
            debug_data: debug_data.to_vec(),
        })
    }
}

impl From<SettingsError> for ErrorCode {
    fn from(value: SettingsError) -> Self {
        match value {
            SettingsError::Length => ErrorCode::FrameSizeError,
            SettingsError::EnablePush => ErrorCode::ProtocolError,
            SettingsError::MaxFrameSize => ErrorCode::ProtocolError,
            SettingsError::InitialWindowSize => ErrorCode::FlowControlError,
        }
    }
}

pub fn rst_stream(stream_id: u32, code: ErrorCode) -> Frame {
    Frame::new(
        FrameType::RstStream,
        Flags::empty(),
        stream_id,
        u32::from(code).to_be_bytes(),
    )
}
pub fn parse_rst_stream(payload: &[u8]) -> Result<ErrorCode, PayloadError> {
    let byte = <[u8; 4]>::try_from(payload).map_err(|_| PayloadError::Length)?;
    let code = u32::from_be_bytes(byte);

    Ok(ErrorCode::from(code))
}

pub fn goaway(last: u32, code: ErrorCode, debug: &[u8]) -> Frame {
    let mut payload = vec![];

    // removing R bit
    let id = last & RESERVED_BIT_MASK;

    payload.extend_from_slice(id.to_be_bytes().as_slice());
    payload.extend_from_slice(u32::from(code).to_be_bytes().as_slice());
    payload.extend_from_slice(debug);

    Frame::new(FrameType::GoAway, Flags::empty(), 0, payload)
}

impl From<u32> for ErrorCode {
    fn from(value: u32) -> Self {
        match value {
            0x0 => ErrorCode::NoError,
            0x1 => ErrorCode::ProtocolError,
            0x2 => ErrorCode::InternalError,
            0x3 => ErrorCode::FlowControlError,
            0x4 => ErrorCode::SettingsTimeout,
            0x5 => ErrorCode::StreamClosed,
            0x6 => ErrorCode::FrameSizeError,
            0x7 => ErrorCode::RefusedStream,
            0x8 => ErrorCode::Cancel,
            0x9 => ErrorCode::CompressionError,
            0xa => ErrorCode::ConnectError,
            0xb => ErrorCode::EnhanceYourCalm,
            0xc => ErrorCode::InadequateSecurity,
            0xd => ErrorCode::Http11Required,
            value => ErrorCode::Unknown(value),
        }
    }
}

impl From<ErrorCode> for u32 {
    fn from(value: ErrorCode) -> Self {
        match value {
            ErrorCode::ProtocolError => 0x1,
            ErrorCode::InternalError => 0x2,
            ErrorCode::FlowControlError => 0x3,
            ErrorCode::SettingsTimeout => 0x4,
            ErrorCode::StreamClosed => 0x5,
            ErrorCode::FrameSizeError => 0x6,
            ErrorCode::RefusedStream => 0x7,
            ErrorCode::Cancel => 0x8,
            ErrorCode::CompressionError => 0x9,
            ErrorCode::ConnectError => 0xa,
            ErrorCode::EnhanceYourCalm => 0xb,
            ErrorCode::InadequateSecurity => 0xc,
            ErrorCode::Http11Required => 0xd,
            ErrorCode::NoError => 0x0,
            ErrorCode::Unknown(value) => value,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http2::frame::{Flags, FrameType};

    // RFC 9113 §7, in wire order.
    const CODES: [(u32, ErrorCode); 14] = [
        (0x0, ErrorCode::NoError),
        (0x1, ErrorCode::ProtocolError),
        (0x2, ErrorCode::InternalError),
        (0x3, ErrorCode::FlowControlError),
        (0x4, ErrorCode::SettingsTimeout),
        (0x5, ErrorCode::StreamClosed),
        (0x6, ErrorCode::FrameSizeError),
        (0x7, ErrorCode::RefusedStream),
        (0x8, ErrorCode::Cancel),
        (0x9, ErrorCode::CompressionError),
        (0xa, ErrorCode::ConnectError),
        (0xb, ErrorCode::EnhanceYourCalm),
        (0xc, ErrorCode::InadequateSecurity),
        (0xd, ErrorCode::Http11Required),
    ];

    // --- 8.1 ErrorCode (RFC 9113 §7) ---

    #[test]
    fn known_codes_are_decoded() {
        for (value, code) in CODES {
            assert_eq!(ErrorCode::from(value), code, "decoding {value:#x}");
        }
    }

    #[test]
    fn known_codes_are_encoded() {
        for (value, code) in CODES {
            assert_eq!(u32::from(code), value, "encoding {code:?}");
        }
    }

    // §7: unknown codes MUST NOT trigger special behavior, but the value is kept.
    #[test]
    fn unknown_codes_are_kept_as_is() {
        assert_eq!(ErrorCode::from(0xe), ErrorCode::Unknown(0xe));
        assert_eq!(ErrorCode::from(u32::MAX), ErrorCode::Unknown(u32::MAX));
    }

    #[test]
    fn unknown_codes_round_trip() {
        for value in [0xe, 0x1234, u32::MAX] {
            assert_eq!(u32::from(ErrorCode::from(value)), value);
        }
    }

    // --- 8.2 RST_STREAM (RFC 9113 §6.4) ---

    #[test]
    fn rst_stream_frame_header() {
        let frame = rst_stream(1, ErrorCode::ProtocolError);

        assert_eq!(frame.frame_type(), FrameType::RstStream);
        assert_eq!(frame.flags(), Flags::empty()); // §6.4: no flags defined
        assert_eq!(frame.stream_id(), 1);
    }

    #[test]
    fn rst_stream_on_the_wire() {
        let bytes = rst_stream(1, ErrorCode::ProtocolError).to_bytes();

        #[rustfmt::skip]
        assert_eq!(bytes, [
            0x00, 0x00, 0x04,       // length = 4
            0x03,                   // type = RST_STREAM
            0x00,                   // flags
            0x00, 0x00, 0x00, 0x01, // stream 1
            0x00, 0x00, 0x00, 0x01, // PROTOCOL_ERROR
        ]);
    }

    #[test]
    fn rst_stream_carries_any_code() {
        let frame = rst_stream(5, ErrorCode::Unknown(0xabcd));

        assert_eq!(frame.payload(), [0x00, 0x00, 0xab, 0xcd]);
    }

    #[test]
    fn rst_stream_payload_is_parsed() {
        assert_eq!(
            parse_rst_stream(&[0x00, 0x00, 0x00, 0x08]),
            Ok(ErrorCode::Cancel)
        );
    }

    #[test]
    fn rst_stream_with_unknown_code_is_parsed() {
        assert_eq!(
            parse_rst_stream(&[0x00, 0x00, 0x00, 0xff]),
            Ok(ErrorCode::Unknown(0xff))
        );
    }

    // §6.4: a length other than 4 octets is a FRAME_SIZE_ERROR.
    #[test]
    fn rst_stream_payload_must_be_4_bytes() {
        for payload in [&[][..], &[0x00; 3], &[0x00; 5]] {
            assert_eq!(
                parse_rst_stream(payload),
                Err(PayloadError::Length),
                "payload of {} bytes",
                payload.len()
            );
        }
    }

    #[test]
    fn rst_stream_round_trip() {
        let frame = rst_stream(3, ErrorCode::RefusedStream);

        assert_eq!(
            parse_rst_stream(frame.payload()),
            Ok(ErrorCode::RefusedStream)
        );
    }

    // --- 8.2 GOAWAY (RFC 9113 §6.8) ---

    // GOAWAY applies to the whole connection: always sent on stream 0.
    #[test]
    fn goaway_frame_header() {
        let frame = goaway(3, ErrorCode::NoError, &[]);

        assert_eq!(frame.frame_type(), FrameType::GoAway);
        assert_eq!(frame.flags(), Flags::empty());
        assert_eq!(frame.stream_id(), 0);
    }

    #[test]
    fn goaway_on_the_wire() {
        let bytes = goaway(3, ErrorCode::CompressionError, b"hpack").to_bytes();

        #[rustfmt::skip]
        assert_eq!(bytes, [
            0x00, 0x00, 0x0d,             // length = 4 + 4 + 5
            0x07,                         // type = GOAWAY
            0x00,                         // flags
            0x00, 0x00, 0x00, 0x00,       // stream 0
            0x00, 0x00, 0x00, 0x03,       // last stream id = 3
            0x00, 0x00, 0x00, 0x09,       // COMPRESSION_ERROR
            b'h', b'p', b'a', b'c', b'k', // debug data
        ]);
    }

    #[test]
    fn goaway_without_debug_data_is_8_bytes() {
        let frame = goaway(0, ErrorCode::NoError, &[]);

        assert_eq!(frame.payload(), [0x00; 8]);
    }

    // §6.8: the reserved bit before Last-Stream-ID MUST be unset when sending.
    #[test]
    fn goaway_clears_the_reserved_bit() {
        let frame = goaway(0x8000_0003, ErrorCode::NoError, &[]);

        assert_eq!(frame.payload()[..4], [0x00, 0x00, 0x00, 0x03]);
    }

    #[test]
    fn goaway_payload_is_parsed() {
        let payload = [
            0x00, 0x00, 0x00, 0x07, // last stream id = 7
            0x00, 0x00, 0x00, 0x0b, // ENHANCE_YOUR_CALM
            b'b', b'y', b'e',
        ];

        assert_eq!(
            GoAway::parse(&payload),
            Ok(GoAway {
                last_stream_id: 7,
                error_code: ErrorCode::EnhanceYourCalm,
                debug_data: b"bye".to_vec(),
            })
        );
    }

    #[test]
    fn goaway_without_debug_data_is_parsed() {
        let payload = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];

        assert_eq!(
            GoAway::parse(&payload),
            Ok(GoAway {
                last_stream_id: 1,
                error_code: ErrorCode::NoError,
                debug_data: vec![],
            })
        );
    }

    // §6.8: the reserved bit MUST be ignored when receiving.
    #[test]
    fn goaway_ignores_the_reserved_bit() {
        let payload = [0x80, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00];

        let parsed = GoAway::parse(&payload).unwrap();

        assert_eq!(parsed.last_stream_id, 5);
    }

    #[test]
    fn goaway_with_unknown_code_is_parsed() {
        let payload = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00];

        let parsed = GoAway::parse(&payload).unwrap();

        assert_eq!(parsed.error_code, ErrorCode::Unknown(0x100));
    }

    // Last-Stream-ID and Error Code are mandatory: at least 8 octets.
    #[test]
    fn goaway_payload_shorter_than_8_bytes_is_an_error() {
        for len in 0..8 {
            assert_eq!(
                GoAway::parse(&vec![0x00; len]),
                Err(PayloadError::Length),
                "payload of {len} bytes"
            );
        }
    }

    #[test]
    fn goaway_round_trip() {
        let frame = goaway(9, ErrorCode::FrameSizeError, b"too big");

        assert_eq!(
            GoAway::parse(frame.payload()),
            Ok(GoAway {
                last_stream_id: 9,
                error_code: ErrorCode::FrameSizeError,
                debug_data: b"too big".to_vec(),
            })
        );
    }
}
