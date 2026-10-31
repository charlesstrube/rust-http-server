# Step 8: proper errors

RFC 9113 §5.4 and §7.

- An `ErrorCode` enum: `NO_ERROR` 0x0, `PROTOCOL_ERROR` 0x1, `INTERNAL_ERROR` 0x2, `FLOW_CONTROL_ERROR` 0x3, `SETTINGS_TIMEOUT` 0x4, `STREAM_CLOSED` 0x5, `FRAME_SIZE_ERROR` 0x6, `REFUSED_STREAM` 0x7, `CANCEL` 0x8, `COMPRESSION_ERROR` 0x9…
- **Stream error** → `RST_STREAM` (§6.4, 4-byte error code) on that stream; the connection goes on.
- **Connection error** → `GOAWAY` (§6.8: last processed stream id + error code), then close.
- Today's `Decision::Close` becomes `Close(ErrorCode)`: the table in `connection.rs` already lists which code goes where.
- `RST_STREAM` received from the client: forget that stream, keep the connection.

Tests: every "closes the connection" test in `http2/mod.rs` now also expects a GOAWAY with the right code.

[← Roadmap](README.md)
