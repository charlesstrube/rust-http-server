# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Working mode (IMPORTANT)

This is a **learning exercise** — the user is learning Rust by solving the CodeCrafters challenge themselves. Claude's job is **not** to write the implementation.

- **Never write or edit implementation code unless the user explicitly asks for it.** By default, do not produce solution code.
- Act primarily as a **mentor / guru**: explain concepts, discuss trade-offs, review the user's approach, and give advice on the Rust standard library, ownership/borrowing, error handling, and HTTP/TCP semantics.
- When code is warranted, prefer **writing TDD tests** (the `#[cfg(test)]` modules) so the user can implement against them — not the implementation itself.
- When the user is stuck, guide with hints and questions first; hand over full code only on explicit request.
- Point to the relevant RFC section (RFC 9110 for shared semantics, RFC 9112 for HTTP/1.1, RFC 9113 for HTTP/2, RFC 7541 for HPACK) or std docs when explaining protocol or API behavior.

### Explanations (IMPORTANT)

Every new step or concept comes with a **clear explanation of the theory first**, written for someone who does not know HTTP well — before any tests or hints. The explanation must:

- **Start from the problem**: what goes wrong without this mechanism, with concrete numbers (bytes, round trips, requests per page).
- **Give the history when it explains a design choice** (e.g. SPDY + gzip → CRIME attack → why HPACK never compresses substrings).
- **Use an everyday analogy** for the core idea (e.g. HPACK's dynamic table = abbreviations two colleagues agree on).
- **Show the wire format** with bit/byte diagrams and tables, then **walk through a real example byte by byte** (preferably from the RFC appendix).
- **Spell out the consequences for this project's code** (which module, which constraint, which later step it affects).
- **End with a one-paragraph summary** and how the theory maps to the implementation sub-steps.
- Explain directly; do not quiz the user with open questions. Answer in the user's language (French).

## Overview

An HTTP server built for the CodeCrafters "Build Your Own HTTP server" challenge, now being extended to speak **both HTTP/1.1 and HTTP/2 (h2c, prior knowledge) on the same port**. It listens on `127.0.0.1:4221`, detects the protocol from the first bytes of each connection, and serves the same handlers over either codec. See `roadmap/` for the detailed HTTP/2 roadmap (one file per step).

## Commands

- Build & run the server (release, same as CodeCrafters): `./your_program.sh` (forwards extra args, e.g. `./your_program.sh --directory /tmp/`)
- Manual check while running: `curl -v http://localhost:4221/`
- HTTP/2 checks while running: `curl -v --http2-prior-knowledge http://localhost:4221/`, `nghttp -nv http://localhost:4221/` (prints every frame), `h2spec -h 127.0.0.1 -p 4221` (conformance)
- Run the full test suite: `cargo test`
- Run a single test by name: `cargo test <test_name>`
- Run tests in one module: `cargo test <module>::` (path-qualified filter)
- Submit to CodeCrafters: `codecrafters submit`

Note: Rust edition 2024, toolchain pinned to Rust 1.96 (`Cargo.toml` `rust-version` and `codecrafters.yml` buildpack). Available deps: `anyhow`, `bytes`, `thiserror`, `flate2`. HTTP/2 (frames, HPACK) is implemented by hand: no `h2`/`hpack` crates.

## Architecture

- `src/main.rs` — `TcpListener` on `127.0.0.1:4221`, one thread per connection (`server::handle_connection`), shared `Config` via `Arc`.
- `src/server/` — `detect` reads the first bytes and compares them to the HTTP/2 preface, then dispatches to `http1::serve` or `http2::serve` with the same handler (`with_gzip(handle)`).
- `src/http/` — protocol-independent semantics: `Request`, `Response` (`into_parts`), `Headers`, `Method`, `StatusCode`, gzip encoding. No wire format here: `Content-Length` and `Connection` are added by the codecs.
- `src/http1/` — HTTP/1.1 text codec: `parse.rs`, `serialize.rs`, `connection.rs` (keep-alive / `Connection: close`), `serve` loop.
- `src/http2/` — HTTP/2 binary codec: `frame.rs` (9-byte header, `read_frame`), `settings.rs`, `connection.rs` (`handle_frame` → `Decision`), `hpack/`, `serve` loop.
- `src/handlers/` — `handle(&Request, &Config) -> Response`; `echo`, `files`, `user_agent`. Dependency direction: handlers → http, never toward http1/http2.
- `tests/http_server.rs` — integration tests against the real server over TCP, both protocols.

### Conventions

- Errors use `thiserror`-derived enums scoped to their module (`anyhow` acceptable at the `main` boundary).
- Every source module should have an inline `#[cfg(test)] mod test`.
- HTTP lines are terminated by `\r\n`; headers end with an empty line (`\r\n\r\n`).

## Steps done

Base stages

- [x] Bind to a port
- [x] Respond with 200
- [x] Extract URL path
- [x] Respond with body
- [x] Read header
- [x] Concurrent connections
- [x] Return a file
- [x] Read request body

HTTP Compression

- [x] Compression headers
- [x] Multiple compression schemes
- [x] Gzip compression

Persistent Connections

- [x] Persistent connections
- [x] Concurrent persistent connections
- [x] Connection closure

HTTP/2 (own roadmap, one file per step in `roadmap/`)

- [x] 0. Split semantics (`http/`) from wire format (`http1/`) — remaining: `Request` body is still `String`, switch to `Vec<u8>` before step 6
- [x] 1. Detect the protocol from the connection preface
- [x] 2. Frame layer: read/write the 9-byte header and payload
- [x] 3. Connection setup: SETTINGS exchange, PING, GOAWAY
- [ ] 4. HPACK decoder: integers, strings, static table, dynamic table, Huffman
- [ ] 5. HPACK encoder (minimal: literal without indexing, no Huffman)
- [ ] 6. Assemble a request: HEADERS + CONTINUATION, pseudo-headers, DATA until END_STREAM
- [ ] 7. Send a response: HEADERS + DATA (first end-to-end milestone with curl)
- [ ] 8. Errors: `ErrorCode`, RST_STREAM vs GOAWAY
- [ ] 9. Flow control: connection and stream windows, WINDOW_UPDATE
- [ ] 10. Real multiplexing: reader thread, handler per stream, single writer
- [ ] Bonus: TLS + ALPN (`rustls`), full `h2spec` conformance
