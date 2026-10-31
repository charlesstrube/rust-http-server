# Step 9: flow control

RFC 9113 §5.2 and §6.9. Only DATA frames count.

- **Sending**: two windows, one for the connection and one per stream, both starting at 65 535 (or the peer's `SETTINGS_INITIAL_WINDOW_SIZE` for streams). Only send DATA up to `min(connection window, stream window)`; wait for `WINDOW_UPDATE` to continue.
- **WINDOW_UPDATE received**: an increment of 0 is a `PROTOCOL_ERROR`; a window above 2^31 − 1 is a `FLOW_CONTROL_ERROR`.
- **A new `SETTINGS_INITIAL_WINDOW_SIZE`** shifts the window of every open stream by the difference, possibly below zero (§6.9.2).
- **Receiving**: send `WINDOW_UPDATE` (connection and stream) as request bodies are consumed, otherwise a large POST stalls.

Tests: serve through `/files` a file **larger than 64 KiB**; without flow control the client stops reading and the transfer hangs. Same for a POST of more than 64 KiB.

[← Roadmap](README.md)
