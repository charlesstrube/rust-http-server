# Step 0: split semantics from wire format ✅

`Request` and `Response` describe *what* is exchanged (method, path, status, headers, body). *How* it travels is the codec's job:

```
src/http/      → semantics: Request, Response, Headers, StatusCode, Method
src/http1/     → text codec: parse, serialize, keep-alive
src/http2/     → binary codec: frames, settings, hpack, connection
src/handlers/  → unchanged, sees only src/http/
```

- `Content-Length` and `Connection` are framing: the HTTP/1.1 serializer adds them, `Response` does not.
- Remaining: `Request` body is still a `String`. DATA frames carry bytes, nothing guarantees UTF-8: switch to `Vec<u8>` before step 6.

Done when: a pure refactor, every existing test still passes.

[← Roadmap](README.md)
