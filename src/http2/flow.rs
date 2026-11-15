use crate::http2::{
    error::{ErrorCode, RESERVED_BIT_MASK},
    frame::{Flags, Frame, FrameType::WindowUpdate},
    hpack::integer::{decode_integer, encode_integer},
};

// Flow control (RFC 9113 §5.2, §6.9).
//
// API expected by the tests below:
//
// 9.1 Window
//
// - `DEFAULT_WINDOW_SIZE: u32 = 65_535` and `MAX_WINDOW_SIZE: u32 = 0x7FFF_FFFF` (2^31 - 1).
// - `Window` (Debug, PartialEq): a credit of DATA octets. It is signed: a new
//   SETTINGS_INITIAL_WINDOW_SIZE can push it below zero (§6.9.2).
//   - `Window::new(initial: u32) -> Window`, and `Default` = `DEFAULT_WINDOW_SIZE`.
//   - `available(&self) -> u32`: what can be sent now, 0 when the window is negative.
//   - `consume(&mut self, len: u32) -> Result<(), FlowError>`: `Exceeded` when
//     `len > available()`, and the window is left unchanged.
//   - `increase(&mut self, increment: u32) -> Result<(), FlowError>`: a WINDOW_UPDATE.
//     `Overflow` above `MAX_WINDOW_SIZE`, and the window is left unchanged.
//   - `apply_delta(&mut self, delta: i64) -> Result<(), FlowError>`: the shift
//     `new_initial - old_initial` of a SETTINGS_INITIAL_WINDOW_SIZE change.
//     `Overflow` above `MAX_WINDOW_SIZE`, and the window is left unchanged.
//
// 9.2 WINDOW_UPDATE frame (§6.9)
//
// - `window_update(stream_id: u32, increment: u32) -> Frame`: no flags, 4-byte
//   payload, reserved bit cleared.
// - `parse_window_update(payload: &[u8]) -> Result<u32, FlowError>`: `Length` when
//   the payload is not 4 bytes, `ZeroIncrement` when the increment is 0 (after
//   ignoring the reserved bit).
//
// Errors
//
// - `FlowError` (thiserror, Debug, PartialEq): `Length`, `ZeroIncrement`,
//   `Overflow`, `Exceeded`.
// - `From<FlowError> for ErrorCode`: Length = FRAME_SIZE_ERROR, ZeroIncrement =
//   PROTOCOL_ERROR, Overflow and Exceeded = FLOW_CONTROL_ERROR. Whether it ends
//   up in a RST_STREAM or a GOAWAY depends on the stream id: that is step 9.4.
const DEFAULT_WINDOW_SIZE: u32 = 65_535;
const MAX_WINDOW_SIZE: u32 = 0x7FFF_FFFF;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum FlowError {
    #[error("Exceeded")]
    Exceeded,
    #[error("Overflow")]
    Overflow,
    #[error("Zero Increment")]
    ZeroIncrement,
    #[error("Length")]
    Length,
}

impl From<FlowError> for ErrorCode {
    fn from(value: FlowError) -> Self {
        match value {
            FlowError::Length => Self::FrameSizeError,
            FlowError::ZeroIncrement => Self::ProtocolError,
            FlowError::Overflow => Self::FlowControlError,
            FlowError::Exceeded => Self::FlowControlError,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Window {
    size: i64,
}

impl Default for Window {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW_SIZE)
    }
}

impl Window {
    pub fn new(size: u32) -> Self {
        Self {
            size: i64::from(size),
        }
    }

    pub fn available(&self) -> u32 {
        if self.size.is_negative() {
            0
        } else {
            u32::try_from(self.size).unwrap_or_default()
        }
    }

    pub fn consume(&mut self, len: u32) -> Result<(), FlowError> {
        let converted = i64::from(len);
        if self.size < converted {
            return Err(FlowError::Exceeded);
        }
        if let Some(size) = self.size.checked_sub(converted) {
            self.size = size;
            Ok(())
        } else {
            Err(FlowError::Overflow)
        }
    }

    pub fn increase(&mut self, len: u32) -> Result<(), FlowError> {
        if let Some(size) = self.size.checked_add(i64::from(len)) {
            self.set_new_size(size)
        } else {
            Err(FlowError::Overflow)
        }
    }

    fn set_new_size(&mut self, size: i64) -> Result<(), FlowError> {
        if size > i64::from(MAX_WINDOW_SIZE) {
            Err(FlowError::Overflow)
        } else {
            self.size = size;
            Ok(())
        }
    }

    pub fn apply_delta(&mut self, delta: i64) -> Result<(), FlowError> {
        if let Some(new_size) = self.size.checked_add(delta) {
            self.set_new_size(new_size)
        } else {
            Err(FlowError::Overflow)
        }
    }
}

pub fn parse_window_update(payload: &[u8]) -> Result<u32, FlowError> {
    let bytes = <[u8; 4]>::try_from(payload).map_err(|_| FlowError::Length)?;
    let raw = u32::from_be_bytes(bytes);

    let increment = raw & RESERVED_BIT_MASK;

    if increment == 0 {
        return Err(FlowError::ZeroIncrement);
    }

    Ok(increment)
}

pub fn window_update(stream_id: u32, increment: u32) -> Frame {
    let test = increment & RESERVED_BIT_MASK;

    Frame::new(WindowUpdate, Flags::empty(), stream_id, test.to_be_bytes())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http2::{
        error::ErrorCode,
        frame::{Flags, FrameType},
    };

    // 9.1: cargo test flow::test::window::
    mod window {
        use super::*;

        // --- 9.1 Window: creation ---

        #[test]
        fn default_window_is_65_535() {
            assert_eq!(Window::default().available(), 65_535);
            assert_eq!(Window::default(), Window::new(DEFAULT_WINDOW_SIZE));
        }

        #[test]
        fn new_window_has_its_initial_size() {
            assert_eq!(Window::new(16_383).available(), 16_383);
            assert_eq!(Window::new(0).available(), 0);
            assert_eq!(Window::new(MAX_WINDOW_SIZE).available(), MAX_WINDOW_SIZE);
        }

        // --- 9.1 Window: consume (sending DATA, or receiving it) ---

        #[test]
        fn consume_reduces_the_window() {
            let mut window = Window::default();

            window.consume(16_384).unwrap();

            assert_eq!(window.available(), 49_151);
        }

        // The scenario of the explanation: 3 full frames, then the 4th is cut to 16 383.
        #[test]
        fn consume_down_to_exactly_zero() {
            let mut window = Window::default();

            for len in [16_384, 16_384, 16_384, 16_383] {
                window.consume(len).unwrap();
            }

            assert_eq!(window.available(), 0);
        }

        // §6.9.1: an empty DATA frame (e.g. a lone END_STREAM) costs nothing.
        #[test]
        fn consuming_zero_is_always_allowed() {
            let mut window = Window::new(0);

            assert_eq!(window.consume(0), Ok(()));
            assert_eq!(window.available(), 0);
        }

        // Receiving side: the peer sent more than we allowed (§6.9.1).
        #[test]
        fn consume_beyond_the_window_is_an_error() {
            let mut window = Window::new(100);

            assert_eq!(window.consume(101), Err(FlowError::Exceeded));
        }

        #[test]
        fn failed_consume_leaves_the_window_unchanged() {
            let mut window = Window::new(100);

            let _ = window.consume(101);

            assert_eq!(window.available(), 100);
        }

        // --- 9.1 Window: increase (WINDOW_UPDATE received) ---

        #[test]
        fn increase_adds_credit() {
            let mut window = Window::new(0);

            window.increase(65_535).unwrap();

            assert_eq!(window.available(), 65_535);
        }

        #[test]
        fn increase_up_to_the_maximum_is_allowed() {
            let mut window = Window::new(MAX_WINDOW_SIZE - 10);

            assert_eq!(window.increase(10), Ok(()));
            assert_eq!(window.available(), MAX_WINDOW_SIZE);
        }

        // §6.9.1: a window above 2^31 - 1 is a FLOW_CONTROL_ERROR.
        #[test]
        fn increase_above_the_maximum_is_an_error() {
            let mut window = Window::new(MAX_WINDOW_SIZE - 10);

            assert_eq!(window.increase(11), Err(FlowError::Overflow));
        }

        // The largest increment on the wire, added to the largest window:
        // the sum does not fit in a u32 and must not wrap around.
        #[test]
        fn increase_does_not_wrap_around() {
            let mut window = Window::new(MAX_WINDOW_SIZE);

            assert_eq!(window.increase(MAX_WINDOW_SIZE), Err(FlowError::Overflow));
        }

        #[test]
        fn failed_increase_leaves_the_window_unchanged() {
            let mut window = Window::new(MAX_WINDOW_SIZE - 10);

            let _ = window.increase(11);

            assert_eq!(window.available(), MAX_WINDOW_SIZE - 10);
        }

        // --- 9.1 Window: apply_delta (SETTINGS_INITIAL_WINDOW_SIZE changed, §6.9.2) ---

        #[test]
        fn positive_delta_grows_the_window() {
            let mut window = Window::new(5_535);

            // 65_535 -> 131_070
            window.apply_delta(65_535).unwrap();

            assert_eq!(window.available(), 71_070);
        }

        // The example of the explanation: 60 000 bytes already sent, then the
        // client lowers INITIAL_WINDOW_SIZE from 65 535 to 16 384.
        #[test]
        fn negative_delta_can_push_the_window_below_zero() {
            let mut window = Window::default();
            window.consume(60_000).unwrap();

            window.apply_delta(16_384 - 65_535).unwrap();

            assert_eq!(window.available(), 0);
        }

        // At -43 616, the sender needs 43 617 octets of credit before sending 1 byte.
        #[test]
        fn negative_window_needs_credit_to_come_back() {
            let mut window = Window::default();
            window.consume(60_000).unwrap();
            window.apply_delta(16_384 - 65_535).unwrap();

            window.increase(43_616).unwrap();
            assert_eq!(window.available(), 0);

            window.increase(1).unwrap();
            assert_eq!(window.available(), 1);
        }

        #[test]
        fn negative_window_refuses_any_data() {
            let mut window = Window::new(0);
            window.apply_delta(-1).unwrap();

            assert_eq!(window.consume(1), Err(FlowError::Exceeded));
        }

        // §6.9.2: a change that makes a window exceed 2^31 - 1 is a FLOW_CONTROL_ERROR.
        #[test]
        fn delta_above_the_maximum_is_an_error() {
            let mut window = Window::new(MAX_WINDOW_SIZE);

            assert_eq!(window.apply_delta(1), Err(FlowError::Overflow));
            assert_eq!(window.available(), MAX_WINDOW_SIZE);
        }

        // The smallest possible window: 0 consumed down from the largest initial
        // size, then INITIAL_WINDOW_SIZE goes from 2^31 - 1 to 0. Still valid.
        #[test]
        fn largest_negative_delta_is_allowed() {
            let mut window = Window::new(0);

            assert_eq!(window.apply_delta(-i64::from(MAX_WINDOW_SIZE)), Ok(()));
            assert_eq!(window.available(), 0);

            window.increase(MAX_WINDOW_SIZE).unwrap();
            assert_eq!(window.available(), 0);
        }
    }

    // 9.2: cargo test flow::test::window_update::
    mod window_update {
        use super::*;

        // --- 9.2 WINDOW_UPDATE: building (RFC 9113 §6.9) ---

        #[test]
        fn window_update_frame_header() {
            let frame = window_update(1, 32_768);

            assert_eq!(frame.frame_type(), FrameType::WindowUpdate);
            assert_eq!(frame.flags(), Flags::empty()); // §6.9: no flags defined
            assert_eq!(frame.stream_id(), 1);
        }

        // The example of the explanation, byte by byte.
        #[test]
        fn window_update_on_the_wire() {
            let bytes = window_update(1, 32_768).to_bytes();

            #[rustfmt::skip]
            assert_eq!(bytes, [
                0x00, 0x00, 0x04,       // length = 4
                0x08,                   // type = WINDOW_UPDATE
                0x00,                   // flags
                0x00, 0x00, 0x00, 0x01, // stream 1
                0x00, 0x00, 0x80, 0x00, // increment = 32 768
            ]);
        }

        // Stream 0 = the connection window.
        #[test]
        fn window_update_for_the_connection() {
            let frame = window_update(0, 65_535);

            assert_eq!(frame.stream_id(), 0);
            assert_eq!(frame.payload(), [0x00, 0x00, 0xff, 0xff]);
        }

        // §6.9: the reserved bit is unset when sending.
        #[test]
        fn window_update_clears_the_reserved_bit() {
            let frame = window_update(1, 0x8000_0001);

            assert_eq!(frame.payload(), [0x00, 0x00, 0x00, 0x01]);
        }

        // --- 9.2 WINDOW_UPDATE: parsing ---

        #[test]
        fn window_update_payload_is_parsed() {
            assert_eq!(parse_window_update(&[0x00, 0x00, 0x80, 0x00]), Ok(32_768));
        }

        #[test]
        fn largest_increment_is_parsed() {
            assert_eq!(
                parse_window_update(&[0x7f, 0xff, 0xff, 0xff]),
                Ok(MAX_WINDOW_SIZE)
            );
        }

        // §6.9: the reserved bit MUST be ignored when receiving.
        #[test]
        fn window_update_ignores_the_reserved_bit() {
            assert_eq!(parse_window_update(&[0x80, 0x00, 0x00, 0x05]), Ok(5));
        }

        // §6.9: an increment of 0 is a PROTOCOL_ERROR.
        #[test]
        fn zero_increment_is_an_error() {
            assert_eq!(
                parse_window_update(&[0x00, 0x00, 0x00, 0x00]),
                Err(FlowError::ZeroIncrement)
            );
        }

        // Only the reserved bit set: the increment itself is still 0.
        #[test]
        fn reserved_bit_alone_is_a_zero_increment() {
            assert_eq!(
                parse_window_update(&[0x80, 0x00, 0x00, 0x00]),
                Err(FlowError::ZeroIncrement)
            );
        }

        // §6.9: a length other than 4 octets is a FRAME_SIZE_ERROR.
        #[test]
        fn window_update_payload_must_be_4_bytes() {
            for payload in [
                &[][..],
                &[0x00, 0x00, 0x01],
                &[0x00, 0x00, 0x00, 0x01, 0x00],
            ] {
                assert_eq!(
                    parse_window_update(payload),
                    Err(FlowError::Length),
                    "payload of {} bytes",
                    payload.len()
                );
            }
        }

        #[test]
        fn window_update_round_trip() {
            let frame = window_update(3, 1_000_000);

            assert_eq!(parse_window_update(frame.payload()), Ok(1_000_000));
        }

        // --- Error codes sent to the peer ---

        #[test]
        fn flow_errors_map_to_error_codes() {
            assert_eq!(
                ErrorCode::from(FlowError::Length),
                ErrorCode::FrameSizeError
            );
            assert_eq!(
                ErrorCode::from(FlowError::ZeroIncrement),
                ErrorCode::ProtocolError
            );
            assert_eq!(
                ErrorCode::from(FlowError::Overflow),
                ErrorCode::FlowControlError
            );
            assert_eq!(
                ErrorCode::from(FlowError::Exceeded),
                ErrorCode::FlowControlError
            );
        }
    }
}
