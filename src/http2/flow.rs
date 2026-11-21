use std::{cmp::min, collections::VecDeque};

use crate::http2::{
    connection::{Decision, handle_frame},
    error::{ErrorCode, RESERVED_BIT_MASK},
    frame::{Flags, Frame, FrameType},
    settings::Settings,
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
// 9.3 SendQueue: the response bodies waiting for credit (§5.2, §6.9.1)
//
// HEADERS are not flow-controlled: `serve` writes them at once (see
// `serialize::serialize_head`) and hands the body to the queue. The queue owns
// the connection window and one window per queued stream.
//
// - `SendQueue` (Default = `SendQueue::new()`): connection window 65 535,
//   initial stream window 65 535.
//   - `push(&mut self, stream_id: u32, body: Vec<u8>)`: queue a body, sends nothing.
//     The stream window starts at the current initial window size. An empty body
//     is ignored: END_STREAM is already on HEADERS.
//   - `poll(&mut self, max_frame_size: u32) -> Vec<Frame>`: every DATA frame the
//     windows allow now. Each frame is `min(max_frame_size, connection window,
//     stream window, rest of the body)` long, END_STREAM on the last one, never
//     an empty frame. Streams are served in push order, but a stream blocked by
//     its own window does not block the next ones.
//   - `is_empty(&self) -> bool`: nothing left to send.
//   - `increase(&mut self, stream_id: u32, increment: u32) -> Result<(), FlowError>`:
//     a WINDOW_UPDATE received. Stream 0 = connection. A stream with nothing
//     queued (finished, or response not ready yet) is ignored: being too
//     cautious never breaks the protocol.
//   - `set_initial_window(&mut self, size: u32) -> Result<(), FlowError>`: a new
//     SETTINGS_INITIAL_WINDOW_SIZE. Shifts every queued stream by
//     `size - previous size`, never the connection window (§6.9.2).
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

    Frame::new(
        FrameType::WindowUpdate,
        Flags::empty(),
        stream_id,
        test.to_be_bytes(),
    )
}
#[derive(Debug)]
struct Pending {
    stream_id: u32, // pour les frames DATA et pour retrouver l'entrée dans increase
    window: Window, // fenêtre propre à ce stream
    body: Vec<u8>,  // le body complet
    sent: usize,    // combien d'octets sont déjà partis
}

impl Pending {
    pub fn remaining(&self) -> &[u8] {
        &self.body[self.sent..]
    }
    pub fn is_done(&self) -> bool {
        self.body.len() == self.sent
    }
    pub fn increase(&mut self, length: usize) {
        self.sent += length;
    }
}

#[derive(Debug)]
pub struct SendQueue {
    connection: Window,         // fenêtre de connexion, 65 535 au départ
    initial_window: u32,        // dernière valeur de SETTINGS_INITIAL_WINDOW_SIZE
    pending: VecDeque<Pending>, // envois en attente,
}

impl Default for SendQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl SendQueue {
    pub fn new() -> Self {
        Self {
            connection: Window::default(),
            initial_window: DEFAULT_WINDOW_SIZE,
            pending: VecDeque::new(),
        }
    }

    pub fn push(&mut self, stream_id: u32, body: Vec<u8>) {
        if body.is_empty() {
            return;
        }
        self.pending.push_back(Pending {
            stream_id,
            body,
            sent: 0,
            window: Window::new(self.initial_window),
        })
    }

    pub fn poll(&mut self, max_frame_size: u32) -> Vec<Frame> {
        let mut frames = Vec::new();
        for pending in self.pending.iter_mut() {
            while !pending.is_done() {
                let first = min(
                    max_frame_size as usize,
                    self.connection.available() as usize,
                );
                let second = min(
                    pending.window.available() as usize,
                    pending.remaining().len(),
                );
                let length = min(first, second);

                if length == 0 {
                    break;
                }

                if self.connection.consume(length as u32).is_err() {
                    break;
                };
                if pending.window.consume(length as u32).is_err() {
                    break;
                };

                let remaining = pending.remaining()[..length].to_vec();

                pending.increase(length);

                let flags = if pending.is_done() {
                    Flags::END_STREAM
                } else {
                    Flags::empty()
                };

                frames.push(Frame::new(
                    FrameType::Data,
                    flags,
                    pending.stream_id,
                    remaining,
                ));
            }
        }

        self.pending.retain(|p| !p.is_done());

        frames
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn increase(&mut self, stream_id: u32, increment: u32) -> Result<(), FlowError> {
        if stream_id == 0 {
            self.connection.increase(increment)?;
        } else {
            for pending in self.pending.iter_mut() {
                if pending.stream_id == stream_id {
                    pending.window.increase(increment)?;
                }
            }
        }

        Ok(())
    }

    pub fn set_initial_window(&mut self, size: u32) -> Result<(), FlowError> {
        let delta = i64::from(size) - i64::from(self.initial_window);

        for pending in self.pending.iter_mut() {
            pending.window.apply_delta(delta)?;
        }
        self.initial_window = size;

        Ok(())
    }

    // 9.4: the stream is closed (RST_STREAM received or sent): drop its body.
    pub fn cancel(&mut self, stream_id: u32) {
        self.pending
            .retain(|pending| pending.stream_id != stream_id);
    }
}

pub fn on_settings(frame: Frame, peer_settings: &mut Settings, queue: &mut SendQueue) -> Decision {
    let is_new_settings = !frame.flags().contains(Flags::ACK);

    let decision = handle_frame(frame, peer_settings);

    if is_new_settings
        && matches!(decision, Decision::Reply(_))
        && queue
            .set_initial_window(peer_settings.initial_window_size())
            .is_err()
    {
        return Decision::Close(ErrorCode::FlowControlError);
    }

    decision
}

pub enum FlowDecision {
    Continue,                   // credit applied: just flush the queue
    ResetStream(ErrorCode),     // RST_STREAM on frame.stream_id(), and cancel it in the queue
    CloseConnection(ErrorCode), // GOAWAY with last_stream_id, then stop
}

pub fn on_window_update(frame: &Frame, queue: &mut SendQueue, last_stream_id: u32) -> FlowDecision {
    let stream_id = frame.stream_id();

    let increment = match parse_window_update(frame.payload()) {
        Ok(increment) => increment,
        Err(FlowError::Length) => return FlowDecision::CloseConnection(ErrorCode::FrameSizeError),
        Err(error) => {
            let error_code = ErrorCode::from(error);
            if stream_id == 0 {
                return FlowDecision::CloseConnection(error_code);
            } else {
                return FlowDecision::ResetStream(error_code);
            }
        }
    };

    if stream_id != 0 && stream_id > last_stream_id {
        return FlowDecision::CloseConnection(ErrorCode::ProtocolError);
    }

    if let Err(error) = queue.increase(stream_id, increment) {
        let error_code = ErrorCode::from(error);
        if stream_id == 0 {
            return FlowDecision::CloseConnection(error_code);
        } else {
            return FlowDecision::ResetStream(error_code);
        }
    };

    return FlowDecision::Continue;
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

    // 9.3: cargo test flow::test::send_queue::
    mod send_queue {
        use super::*;
        use crate::http2::frame::Frame;

        const MAX: u32 = 16_384;

        // Non-UTF-8 bytes on purpose, like a file or a gzipped body.
        fn body(len: usize) -> Vec<u8> {
            (0..len).map(|i| (i % 251) as u8).collect()
        }

        // (stream id, payload length) of each frame, checking they are all DATA.
        fn sizes(frames: &[Frame]) -> Vec<(u32, usize)> {
            frames
                .iter()
                .map(|frame| {
                    assert_eq!(frame.frame_type(), FrameType::Data);
                    (frame.stream_id(), frame.payload().len())
                })
                .collect()
        }

        fn ends_stream(frame: &Frame) -> bool {
            frame.flags().contains(Flags::END_STREAM)
        }

        // Which frames carry END_STREAM, in order.
        fn ends(frames: &[Frame]) -> Vec<bool> {
            frames.iter().map(ends_stream).collect()
        }

        fn sent(frames: &[Frame]) -> Vec<u8> {
            frames
                .iter()
                .flat_map(|frame| frame.payload().to_vec())
                .collect()
        }

        fn total(frames: &[Frame]) -> usize {
            frames.iter().map(|frame| frame.payload().len()).sum()
        }

        // A 100 000-byte body on stream 1, polled once: both windows are at 0.
        fn exhausted() -> SendQueue {
            let mut queue = SendQueue::new();
            queue.push(1, body(100_000));
            assert_eq!(total(&queue.poll(MAX)), 65_535);
            queue
        }

        // --- Pushing and polling ---

        #[test]
        fn new_queue_is_empty() {
            let mut queue = SendQueue::new();

            assert!(queue.is_empty());
            assert_eq!(queue.poll(MAX), vec![]);
        }

        #[test]
        fn default_is_new() {
            let mut queue = SendQueue::default();
            queue.push(1, body(100_000));

            assert_eq!(total(&queue.poll(MAX)), 65_535);
        }

        // Sending happens in poll only.
        #[test]
        fn push_sends_nothing() {
            let mut queue = SendQueue::new();

            queue.push(1, b"abc".to_vec());

            assert!(!queue.is_empty());
        }

        #[test]
        fn small_body_is_one_data_frame_that_ends_the_stream() {
            let mut queue = SendQueue::new();
            queue.push(1, b"abc".to_vec());

            let frames = queue.poll(MAX);

            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].frame_type(), FrameType::Data);
            assert_eq!(frames[0].stream_id(), 1);
            assert_eq!(frames[0].flags(), Flags::END_STREAM);
            assert_eq!(frames[0].payload(), b"abc");
            assert!(queue.is_empty());
        }

        // END_STREAM is already on HEADERS: no empty DATA frame afterwards.
        #[test]
        fn empty_body_is_ignored() {
            let mut queue = SendQueue::new();

            queue.push(1, vec![]);

            assert!(queue.is_empty());
            assert_eq!(queue.poll(MAX), vec![]);
        }

        #[test]
        fn nothing_is_sent_twice() {
            let mut queue = SendQueue::new();
            queue.push(1, b"abc".to_vec());
            queue.poll(MAX);

            assert_eq!(queue.poll(MAX), vec![]);
        }

        // --- Frame size (RFC 9113 §4.2) ---

        // 40_000 = 16_384 + 16_384 + 7_232, END_STREAM only on the last one.
        #[test]
        fn body_is_split_at_max_frame_size() {
            let mut queue = SendQueue::new();
            queue.push(1, body(40_000));

            let frames = queue.poll(MAX);

            assert_eq!(sizes(&frames), vec![(1, 16_384), (1, 16_384), (1, 7_232)]);
            assert_eq!(ends(&frames), vec![false, false, true]);
            assert_eq!(sent(&frames), body(40_000));
        }

        // The peer's SETTINGS_MAX_FRAME_SIZE can change: it is given at each poll.
        #[test]
        fn uses_the_given_max_frame_size() {
            let mut queue = SendQueue::new();
            queue.push(1, body(40_000));

            assert_eq!(sizes(&queue.poll(32_768)), vec![(1, 32_768), (1, 7_232)]);
        }

        // A body of exactly the window: everything goes, the last frame is cut to 16 383.
        #[test]
        fn body_of_exactly_the_window_is_sent_entirely() {
            let mut queue = SendQueue::new();
            queue.push(1, body(65_535));

            let frames = queue.poll(MAX);

            assert_eq!(
                sizes(&frames),
                vec![(1, 16_384), (1, 16_384), (1, 16_384), (1, 16_383)]
            );
            assert_eq!(ends(&frames), vec![false, false, false, true]);
            assert!(queue.is_empty());
        }

        // A stream window smaller than a frame cuts the frame.
        #[test]
        fn frame_is_cut_by_the_stream_window() {
            let mut queue = SendQueue::new();
            queue.set_initial_window(1_000).unwrap();
            queue.push(1, body(5_000));

            let frames = queue.poll(MAX);

            assert_eq!(sizes(&frames), vec![(1, 1_000)]);
            assert_eq!(ends(&frames), vec![false]);
        }

        // --- Blocking and resuming: the scenario of the explanation ---

        #[test]
        fn sending_stops_when_the_windows_are_empty() {
            let mut queue = SendQueue::new();
            queue.push(1, body(100_000));

            let frames = queue.poll(MAX);

            assert_eq!(
                sizes(&frames),
                vec![(1, 16_384), (1, 16_384), (1, 16_384), (1, 16_383)]
            );
            assert_eq!(ends(&frames), vec![false, false, false, false]);
            assert!(!queue.is_empty());
            assert_eq!(queue.poll(MAX), vec![]);
        }

        // Connection credit alone: the stream window is still at 0.
        #[test]
        fn connection_credit_alone_does_not_unblock_a_stream() {
            let mut queue = exhausted();

            queue.increase(0, 65_535).unwrap();

            assert_eq!(queue.poll(MAX), vec![]);
        }

        // Stream credit alone: the connection window is still at 0.
        #[test]
        fn stream_credit_alone_does_not_unblock_a_stream() {
            let mut queue = exhausted();

            queue.increase(1, 65_535).unwrap();

            assert_eq!(queue.poll(MAX), vec![]);
        }

        // 100 000 - 65 535 = 34 465 = 16 384 + 16 384 + 1 697.
        #[test]
        fn both_credits_resume_the_stream() {
            let mut queue = SendQueue::new();
            queue.push(1, body(100_000));
            let mut frames = queue.poll(MAX);

            queue.increase(0, 65_535).unwrap();
            queue.increase(1, 65_535).unwrap();
            let rest = queue.poll(MAX);

            assert_eq!(sizes(&rest), vec![(1, 16_384), (1, 16_384), (1, 1_697)]);
            assert_eq!(ends(&rest), vec![false, false, true]);
            assert!(queue.is_empty());

            frames.extend(rest);
            assert_eq!(sent(&frames), body(100_000));
        }

        // Credit is used up exactly: a small increment sends a small frame.
        #[test]
        fn small_credit_sends_a_small_frame() {
            let mut queue = exhausted();

            queue.increase(0, 10).unwrap();
            queue.increase(1, 10).unwrap();

            assert_eq!(sizes(&queue.poll(MAX)), vec![(1, 10)]);
            assert_eq!(queue.poll(MAX), vec![]);
        }

        // --- Several streams (RFC 9113 §5.2.1) ---

        // The connection window is shared: 40 000 + 25 535 = 65 535.
        #[test]
        fn streams_share_the_connection_window() {
            let mut queue = SendQueue::new();
            queue.push(1, body(40_000));
            queue.push(3, body(40_000));

            let frames = queue.poll(MAX);

            assert_eq!(
                sizes(&frames),
                vec![
                    (1, 16_384),
                    (1, 16_384),
                    (1, 7_232),
                    (3, 16_384),
                    (3, 9_151)
                ]
            );
            assert_eq!(ends(&frames), vec![false, false, true, false, false]);

            queue.increase(0, 14_465).unwrap();
            let rest = queue.poll(MAX);

            assert_eq!(sizes(&rest), vec![(3, 14_465)]);
            assert_eq!(ends(&rest), vec![true]);
            assert!(queue.is_empty());
        }

        #[test]
        fn streams_are_served_in_push_order() {
            let mut queue = SendQueue::new();
            queue.push(3, b"abc".to_vec());
            queue.push(1, b"def".to_vec());

            let frames = queue.poll(MAX);

            assert_eq!(sizes(&frames), vec![(3, 3), (1, 3)]);
            assert_eq!(ends(&frames), vec![true, true]);
        }

        // The whole point of per-stream windows: stream 1 waits, stream 3 goes.
        #[test]
        fn a_blocked_stream_does_not_block_the_others() {
            let mut queue = exhausted();
            queue.increase(0, 100_000).unwrap();

            queue.push(3, b"abc".to_vec());
            let frames = queue.poll(MAX);

            assert_eq!(sizes(&frames), vec![(3, 3)]);
            assert_eq!(ends(&frames), vec![true]);
            assert!(!queue.is_empty()); // stream 1 still waits
        }

        // --- WINDOW_UPDATE (RFC 9113 §6.9) ---

        // The response may not be queued yet, or the stream is already finished.
        #[test]
        fn credit_for_a_stream_without_queued_body_is_ignored() {
            let mut queue = SendQueue::new();

            assert_eq!(queue.increase(5, 100), Ok(()));
            assert!(queue.is_empty());
        }

        // §6.9: a WINDOW_UPDATE can arrive just after END_STREAM was sent.
        #[test]
        fn credit_for_a_finished_stream_is_ignored() {
            let mut queue = SendQueue::new();
            queue.push(1, b"abc".to_vec());
            queue.poll(MAX);

            assert_eq!(queue.increase(1, 10), Ok(()));
            assert_eq!(queue.poll(MAX), vec![]);
        }

        // 65 535 + 2^31 - 1 > 2^31 - 1.
        #[test]
        fn connection_window_overflow_is_an_error() {
            let mut queue = SendQueue::new();

            assert_eq!(queue.increase(0, MAX_WINDOW_SIZE), Err(FlowError::Overflow));
        }

        #[test]
        fn stream_window_overflow_is_an_error() {
            let mut queue = SendQueue::new();
            queue.push(1, b"abc".to_vec());

            assert_eq!(queue.increase(1, MAX_WINDOW_SIZE), Err(FlowError::Overflow));
        }

        // --- SETTINGS_INITIAL_WINDOW_SIZE (RFC 9113 §6.9.2) ---

        // The example of the explanation: the stream window falls to -49 151.
        #[test]
        fn lower_initial_window_pushes_a_queued_stream_below_zero() {
            let mut queue = exhausted();
            queue.increase(0, 100_000).unwrap();

            queue.set_initial_window(16_384).unwrap();
            queue.increase(1, 49_151).unwrap();
            assert_eq!(queue.poll(MAX), vec![]);

            queue.increase(1, 1).unwrap();
            assert_eq!(sizes(&queue.poll(MAX)), vec![(1, 1)]);
        }

        // 0 + (85 535 - 65 535) = 20 000 = 16 384 + 3 616.
        #[test]
        fn higher_initial_window_unblocks_a_queued_stream() {
            let mut queue = exhausted();
            queue.increase(0, 100_000).unwrap();

            queue.set_initial_window(85_535).unwrap();

            assert_eq!(sizes(&queue.poll(MAX)), vec![(1, 16_384), (1, 3_616)]);
        }

        // The shift is computed from the previous setting, not from 65 535.
        #[test]
        fn successive_settings_shift_from_the_previous_value() {
            let mut queue = SendQueue::new();
            queue.push(1, body(10_000));

            queue.set_initial_window(1_000).unwrap();
            queue.set_initial_window(3_000).unwrap();

            assert_eq!(sizes(&queue.poll(MAX)), vec![(1, 3_000)]);
        }

        // §6.9.2: only stream windows move; the connection window stays at 65 535.
        #[test]
        fn initial_window_does_not_change_the_connection_window() {
            let mut queue = SendQueue::new();
            queue.set_initial_window(100_000).unwrap();
            queue.push(1, body(200_000));

            assert_eq!(total(&queue.poll(MAX)), 65_535);
        }

        // §6.9.2: a change that makes a stream window exceed 2^31 - 1 is a FLOW_CONTROL_ERROR.
        #[test]
        fn initial_window_overflow_is_an_error() {
            let mut queue = SendQueue::new();
            queue.push(1, b"abc".to_vec());
            queue.increase(1, MAX_WINDOW_SIZE - 65_535).unwrap();

            assert_eq!(queue.set_initial_window(65_536), Err(FlowError::Overflow));
        }

        // --- 9.4 cancel: the stream was reset ---

        #[test]
        fn cancel_drops_the_queued_body() {
            let mut queue = SendQueue::new();
            queue.push(1, body(100));

            queue.cancel(1);

            assert!(queue.is_empty());
            assert_eq!(queue.poll(MAX), vec![]);
        }

        #[test]
        fn cancel_keeps_the_other_streams() {
            let mut queue = SendQueue::new();
            queue.push(1, body(100));
            queue.push(3, b"abc".to_vec());

            queue.cancel(1);

            assert_eq!(sizes(&queue.poll(MAX)), vec![(3, 3)]);
        }

        // A blocked stream is cancelled: credit for it is ignored afterwards.
        #[test]
        fn cancelled_stream_sends_nothing_after_credit() {
            let mut queue = exhausted();

            queue.cancel(1);
            queue.increase(0, 65_535).unwrap();

            assert_eq!(queue.increase(1, 65_535), Ok(()));
            assert_eq!(queue.poll(MAX), vec![]);
        }

        #[test]
        fn cancel_of_an_unknown_stream_does_nothing() {
            let mut queue = SendQueue::new();
            queue.push(1, b"abc".to_vec());

            queue.cancel(5);

            assert_eq!(sizes(&queue.poll(MAX)), vec![(1, 3)]);
        }
    }
}
