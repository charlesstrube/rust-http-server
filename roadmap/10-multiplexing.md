# Step 10: real multiplexing

Until now, streams can be handled one after the other. To truly run them in parallel:

```
reader thread  → reads frames, routes them by stream id
handler threads → one per complete request, calls the handler
writer thread  → the only one that writes to the socket, receives work through an mpsc channel
```

- **The HPACK encoder lives in the writer.** Header blocks must be encoded in the exact order they go on the wire, otherwise the client's dynamic table gets out of sync. Same rule for the decoder on the reader side.
- Frames of different streams may interleave, but a HEADERS + CONTINUATION sequence may not be interrupted.
- Respect `SETTINGS_MAX_CONCURRENT_STREAMS` if you announce one (`REFUSED_STREAM` beyond).
- Flow control windows are now shared between threads: the writer is the natural owner.

Tests: `curl --http2-prior-knowledge --parallel --parallel-immediate` with several URLs, or `h2load -n 100 -c 1 -m 10 http://localhost:4221/echo/abc` (100 requests, 1 connection, 10 streams at once).

[← Roadmap](README.md)
