# Step 9: flow control

RFC 9113 §5.2 and §6.9. Only DATA frames count, padding included.

- **Sending**: two windows, one for the connection and one per stream, both starting at 65 535 (or the peer's `SETTINGS_INITIAL_WINDOW_SIZE` for streams). Only send DATA up to `min(max frame size, connection window, stream window, remaining body)`; keep reading frames while waiting for `WINDOW_UPDATE`.
- **WINDOW_UPDATE received**: an increment of 0 is a `PROTOCOL_ERROR`; a window above 2^31 − 1 is a `FLOW_CONTROL_ERROR` (RST_STREAM on a stream, GOAWAY on stream 0).
- **A new `SETTINGS_INITIAL_WINDOW_SIZE`** shifts the window of every open stream by the difference, possibly below zero (§6.9.2). The connection window is not affected.
- **Receiving**: send `WINDOW_UPDATE` (connection and stream) as request bodies are consumed, otherwise a large POST stalls. DATA ignored on reset or closed streams still counts against the connection window.

## Sub-steps

| | Sub-step | Where | Tests |
|---|---|---|---|
| 9.1 | `Window`: `new`, `available`, `consume`, `increase`, `apply_delta` (signed, max 2^31 − 1) | `src/http2/flow.rs` | written ✅ |
| 9.2 | `window_update` / `parse_window_update`, `FlowError` → `ErrorCode` | `src/http2/flow.rs` | written ✅ |
| 9.3 | `SendQueue` (`push`, `poll`, `increase`, `set_initial_window`) + `serialize_head` | `flow.rs`, `serialize.rs` | written ✅ |
| 9.4 | React to `WINDOW_UPDATE` and `INITIAL_WINDOW_SIZE`, error table | `mod.rs` | |
| 9.5 | Receiving: check our window, give credit back (also for ignored DATA) | `assembler.rs`, `mod.rs` | |
| 9.6 | Integration: 100 000-byte file, raw client without WINDOW_UPDATE gets exactly 65 535 bytes, then the rest | `tests/http_server.rs` | |

Manual check: `nghttp -nv -w 14 -W 14 http://localhost:4221/files/big` (windows of 2^14 − 1 = 16 383 bytes).

[← Roadmap](README.md)
