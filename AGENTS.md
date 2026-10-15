# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Working mode (IMPORTANT)

This is a **learning exercise** — the user is learning Rust by solving the CodeCrafters challenge themselves. Claude's job is **not** to write the implementation.

- **Never write or edit implementation code unless the user explicitly asks for it.** By default, do not produce solution code.
- Act primarily as a **mentor / guru**: explain concepts, discuss trade-offs, review the user's approach, and give advice on the Rust standard library, ownership/borrowing, error handling, and HTTP/TCP semantics.
- When code is warranted, prefer **writing TDD tests** (the `#[cfg(test)]` modules) so the user can implement against them — not the implementation itself.
- When the user is stuck, guide with hints and questions first; hand over full code only on explicit request.
- Point to the relevant RFC section (RFC 9110 / 9112 for HTTP/1.1) or std docs when explaining protocol or API behavior.

## Overview

An HTTP/1.1 server built for the CodeCrafters "Build Your Own HTTP server" challenge. It listens on `127.0.0.1:4221`, accepts TCP connections, parses raw HTTP requests, and writes back HTTP responses.

## Commands

- Build & run the server (release, same as CodeCrafters): `./your_program.sh` (forwards extra args, e.g. `./your_program.sh --directory /tmp/`)
- Manual check while running: `curl -v http://localhost:4221/`
- Run the full test suite: `cargo test`
- Run a single test by name: `cargo test <test_name>`
- Run tests in one module: `cargo test <module>::` (path-qualified filter)
- Submit to CodeCrafters: `codecrafters submit`

Note: Rust edition 2024, toolchain pinned to Rust 1.96 (`Cargo.toml` `rust-version` and `codecrafters.yml` buildpack). Available deps: `anyhow`, `bytes`, `thiserror`.

## Architecture

Currently everything lives in `src/main.rs`: a `TcpListener` bound to `127.0.0.1:4221` iterating over `listener.incoming()`. The code is at an early stage — **update this section as modules are introduced** (e.g. request parsing, response building, routing, concurrency model).

### Conventions

- Errors use `thiserror`-derived enums scoped to their module (`anyhow` acceptable at the `main` boundary).
- Every source module should have an inline `#[cfg(test)] mod test`.
- HTTP lines are terminated by `\r\n`; headers end with an empty line (`\r\n\r\n`).

## Steps done

Base stages

- [x] Bind to a port
- [x] Respond with 200
- [ ] Extract URL path
- [ ] Respond with body
- [ ] Read header
- [ ] Concurrent connections
- [ ] Return a file
- [ ] Read request body

HTTP Compression

- [ ] Compression headers
- [ ] Multiple compression schemes
- [ ] Gzip compression

Persistent Connections

- [ ] Persistent connections
- [ ] Concurrent persistent connections
- [ ] Connection closure
