# Step 7: send a response (first end-to-end milestone)

- Call the handler, exactly as HTTP/1.1 does.
- Encode `:status` first, then the headers: lowercase names, **drop** `connection` and other connection-specific headers (§8.2.2). `content-length` is allowed but no longer frames anything.
- HEADERS with `END_HEADERS`, plus `END_STREAM` if the body is empty.
- The body in DATA frames of at most the **peer's** `max_frame_size`, `END_STREAM` on the last one.
- Gzip keeps working unchanged: it acts on the body.

Done when: `curl --http2-prior-knowledge http://localhost:4221/echo/abc` prints `abc`, and `/user-agent`, `/files` (GET and POST) work too. Run `h2spec` for the first time: its failures list the RFC sections still missing.

[← Roadmap](README.md)
