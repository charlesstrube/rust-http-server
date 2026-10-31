# Step 6: assemble a request

- **HEADERS** (§6.2), on an odd stream id:
  - flag `PADDED` (0x8): first byte = pad length, remove the padding at the end;
  - flag `PRIORITY` (0x20): skip 5 bytes (deprecated, ignore the content);
  - without `END_HEADERS` (0x4), **CONTINUATION** frames (§6.10) follow on the same stream, with nothing in between: concatenate the blocks, then decode once.
- **Pseudo-headers** (§8.3.1): `:method`, `:scheme`, `:path` are mandatory, `:authority` optional. They become `Request::new(...)`. Reject (`PROTOCOL_ERROR`, stream error): an unknown or duplicated pseudo-header, a pseudo-header after a regular header, an uppercase name, a connection-specific header (`connection`, `keep-alive`, `transfer-encoding`…; `te` is only allowed as `trailers`).
- **DATA** (§6.1): append to the body until `END_STREAM` (0x1). `END_STREAM` may also be on HEADERS (a GET without body).
- **Stream ids** (§5.1.1): client ids are odd and strictly increasing. A smaller or even id is a `PROTOCOL_ERROR`.
- **Simplified stream states** (§5.1): idle → open (HEADERS) → half-closed remote (END_STREAM received) → closed (END_STREAM sent). DATA on a stream that is not open is a `STREAM_CLOSED` error.

Tests: unit tests on a `Cursor`: a GET in a single HEADERS frame, a POST with two DATA frames, a header block split over CONTINUATION, padding.

[← Roadmap](README.md)
