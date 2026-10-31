# Step 1: detect the protocol ✅

The client preface is 24 bytes: `PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n` (RFC 9113 §3.4). Read byte by byte until a mismatch (HTTP/1.1) or the full preface (HTTP/2). On HTTP/1.1, the bytes already consumed are put back in front of the reader (`Cursor::new(consumed).chain(reader)`).

Tests:

- the preface goes to HTTP/2;
- a normal `GET` goes to HTTP/1.1, with no byte lost;
- a request shorter than 24 bytes (`GET / HTTP/1.1\r\n\r\n` is 18) does not block detection;
- EOF before any byte is not an error.

[← Roadmap](README.md)
