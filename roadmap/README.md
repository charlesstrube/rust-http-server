# HTTP/2 roadmap

Goal: serve **HTTP/1.1 and HTTP/2 on the same port** (`127.0.0.1:4221`), with the same handlers. HTTP/2 is spoken in cleartext with *prior knowledge* (h2c): no TLS, no `Upgrade: h2c` (deprecated by RFC 9113 §3.1). Frames and HPACK are written by hand, without the `h2` crate.

References:

- [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110): HTTP semantics, shared by both versions (methods, status codes, headers)
- [RFC 9112](https://www.rfc-editor.org/rfc/rfc9112): HTTP/1.1 wire format
- [RFC 9113](https://www.rfc-editor.org/rfc/rfc9113): HTTP/2 wire format
- [RFC 7541](https://www.rfc-editor.org/rfc/rfc7541): HPACK, header compression

Tools:

```sh
brew install nghttp2 h2spec
curl -v --http2-prior-knowledge http://localhost:4221/echo/abc
nghttp -nv http://localhost:4221/echo/abc   # prints every frame
h2spec -h 127.0.0.1 -p 4221                 # conformance, useful from step 7
```

Each step has its own file below and ends with green tests (`cargo test`). Steps 0 to 7 give a working HTTP/2 server; steps 8 to 10 make it correct and truly concurrent.

## Steps

- [Step 0: split semantics from wire format ✅](00-split-semantics.md)
- [Step 1: detect the protocol ✅](01-detect-protocol.md)
- [Step 2: frame layer ✅](02-frame-layer.md)
- [Step 3: connection setup ✅](03-connection-setup.md)
- [Step 4: HPACK decoder](04-hpack-decoder.md)
- [Step 5: HPACK encoder (minimal)](05-hpack-encoder.md)
- [Step 6: assemble a request](06-assemble-request.md)
- [Step 7: send a response (first end-to-end milestone)](07-send-response.md)
- [Step 8: proper errors](08-errors.md)
- [Step 9: flow control](09-flow-control.md)
- [Step 10: real multiplexing](10-multiplexing.md)
- [Bonus](bonus.md)
