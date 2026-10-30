use crate::http2::{
    frame::{Flags, Frame, FrameType},
    settings::Settings,
};

pub enum Decision {
    Continue,     // nothing to write
    Reply(Frame), // write this frame, then keep reading
    Close,        // connection error or GOAWAY: stop
}

// Connection-level frame handling (RFC 9113). Rules are evaluated top to
// bottom within each frame type: validation first, then processing.
// The error code column is unused for now (Close = return); step 8 will
// turn it into a GOAWAY.
//
// | Frame    | Condition                     | Decision                       | Error code               | RFC     |
// |----------|-------------------------------|--------------------------------|--------------------------|---------|
// | (first)  | not a SETTINGS                | Close                          | PROTOCOL_ERROR           | §3.4    |
// | SETTINGS | stream_id != 0                | Close                          | PROTOCOL_ERROR           | §6.5    |
// | SETTINGS | ACK + payload not empty       | Close                          | FRAME_SIZE_ERROR         | §6.5    |
// | SETTINGS | ACK                           | Continue                       | -                        | §6.5    |
// | SETTINGS | apply() fails                 | Close (never ACK bad settings) | depends on SettingsError | §6.5.2  |
// | SETTINGS | otherwise                     | Reply(SETTINGS + ACK, empty)   | -                        | §6.5.3  |
// | PING     | stream_id != 0                | Close                          | PROTOCOL_ERROR           | §6.7    |
// | PING     | payload.len() != 8            | Close                          | FRAME_SIZE_ERROR         | §6.7    |
// | PING     | ACK                           | Continue                       | -                        | §6.7    |
// | PING     | otherwise                     | Reply(PING + ACK, same payload)| -                        | §6.7    |
// | GOAWAY   | any                           | Close                          | -                        | §6.8    |
// | Unknown  | any                           | Continue (MUST be ignored)     | -                        | §4.1    |
// | others   | any                           | Continue (until steps 6-9)     | -                        | -       |
//
// SettingsError -> error code: InvalidLength = FRAME_SIZE_ERROR,
// InvalidEnablePush / InvalidMaxFrameSize = PROTOCOL_ERROR,
// InvalidInitialWindowSize = FLOW_CONTROL_ERROR.

pub fn handle_frame(frame: Frame, peer_settings: &mut Settings) -> Decision {
    match frame.frame_type() {
        FrameType::Settings if frame.stream_id() != &0 => Decision::Close,
        FrameType::Settings
            if frame.flags().contains(Flags::ACK) && !frame.payload().is_empty() =>
        {
            Decision::Close
        }
        FrameType::Settings if frame.flags().contains(Flags::ACK) => Decision::Continue,
        FrameType::Settings => {
            if peer_settings.apply(frame.payload()).is_err() {
                return Decision::Close;
            }

            Decision::Reply(Frame::new(FrameType::Settings, Flags::ACK, 0, []))
        }
        FrameType::Ping if frame.stream_id() != &0 => Decision::Close,
        FrameType::Ping if frame.payload().len() != 8 => Decision::Close,
        FrameType::Ping if frame.flags().contains(Flags::ACK) => Decision::Continue,
        FrameType::Ping => {
            Decision::Reply(Frame::new(FrameType::Ping, Flags::ACK, 0, frame.payload()))
        }
        FrameType::GoAway => Decision::Close,
        FrameType::Unknown(_) => Decision::Continue,
        _ => Decision::Continue,
    }
}
