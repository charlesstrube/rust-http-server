# Step 2: frame layer ✅

RFC 9113 §4.1. Every frame starts with a fixed 9-byte header:

```
+-----------------------------------------------+
|                 Length (24)                   |
+---------------+---------------+---------------+
|   Type (8)    |   Flags (8)   |
+-+-------------+---------------+-------------------------------+
|R|                 Stream Identifier (31)                      |
+=+=============================================================+
|                   Frame Payload (Length)                    ...
+---------------------------------------------------------------+
```

- Unknown frame types MUST be ignored: keep them as `FrameType::Unknown(u8)`, never reject them.
- Mask the reserved bit: `stream_id & 0x7FFF_FFFF`.
- A payload larger than the `SETTINGS_MAX_FRAME_SIZE` *we* announced (default 16 384) is a `FRAME_SIZE_ERROR`.
- EOF before the first header byte = end of connection (`Ok(None)`); EOF in the middle of a frame = truncated (error).

Tests: round trip serialize → parse, 24-bit length, R bit set, truncated header, truncated payload, oversized frame. Everything runs on a `Cursor`, no network.

[← Roadmap](README.md)
