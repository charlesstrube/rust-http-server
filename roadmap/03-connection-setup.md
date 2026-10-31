# Step 3: connection setup ✅

- **Server preface**: our own SETTINGS frame, written first, without waiting for the client (§3.4). It can be empty: the RFC defaults suit us.
- **The first client frame must be SETTINGS**, otherwise `PROTOCOL_ERROR`.
- **SETTINGS** (§6.5): 6-byte entries (16-bit id, 32-bit value). Validate before acknowledging (§6.5.2):
  - `ENABLE_PUSH` must be 0 or 1;
  - `INITIAL_WINDOW_SIZE` must stay ≤ 2^31 − 1 (`FLOW_CONTROL_ERROR`);
  - `MAX_FRAME_SIZE` must be in 16 384 ..= 16 777 215;
  - unknown ids are ignored.
  Then reply with an empty SETTINGS + ACK. Never acknowledge an ACK.
- **PING** (§6.7): stream 0, payload of exactly 8 bytes, answered with the same payload + ACK. A PING ACK is not answered.
- **GOAWAY** (§6.8): stop reading.

Milestone: `nghttp -nv` shows the SETTINGS exchange, then stalls because HEADERS are not handled yet.

[← Roadmap](README.md)
