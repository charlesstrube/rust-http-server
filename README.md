[![progress-banner](https://backend.codecrafters.io/progress/http-server/5b7c201f-4b0a-4ef9-8db6-a874aea410de)](https://app.codecrafters.io/users/charlesstrube?r=2qF)

This is a starting point for Rust solutions to the
["Build Your Own HTTP server" Challenge](https://app.codecrafters.io/courses/http-server/overview).

[HTTP](https://en.wikipedia.org/wiki/Hypertext_Transfer_Protocol) is the
protocol that powers the web. In this challenge, you'll build a HTTP/1.1 server
that is capable of serving multiple clients.

Along the way you'll learn about TCP servers,
[HTTP request syntax](https://www.w3.org/Protocols/rfc2616/rfc2616-sec5.html),
and more.

**Note**: If you're viewing this repo on GitHub, head over to
[codecrafters.io](https://codecrafters.io) to try the challenge.

# Passing the first stage

The entry point for your HTTP server implementation is in `src/main.rs`. Study
and uncomment the relevant code, and then run the command below to execute the
tests on our servers:

```sh
codecrafters submit
```

Time to move on to the next stage!

# Stage 2 & beyond

Note: This section is for stages 2 and beyond.

1. Ensure you have `cargo (1.96)` installed locally
1. Run `./your_program.sh` to run your program, which is implemented in
   `src/main.rs`. This command compiles your Rust project, so it might be slow
   the first time you run it. Subsequent runs will be fast.
1. Run `codecrafters submit` to submit your solution to CodeCrafters. Test
   output will be streamed to your terminal.


# Architecture

```
src/
  ├── main.rs              // entry point: reads the config, binds 127.0.0.1:4221, one thread per connection
  ├── config.rs            // struct Config { directory: Option<PathBuf> }, parsed from the CLI args
  ├── test_support.rs      // (tests only) TempDir: temporary directory removed on Drop
  ├── server/              // the only module that knows about both protocols
  │   ├── mod.rs           // handle_connection: detect() reads the first bytes, dispatches to http1 or http2
  │   └── protocol.rs      // enum Protocol { Http1, Http2 }
  ├── http/                // shared HTTP semantics (RFC 9110), independent of the wire format
  │   ├── method.rs        // enum Method { Get, Post, Put } + parsing
  │   ├── status.rs        // enum StatusCode -> code + reason phrase
  │   ├── request.rs       // struct Request { method, path, headers, body } + RequestError
  │   ├── response.rs      // struct Response { status, headers, body }
  │   └── headers/
  │       ├── mod.rs       // struct Headers(Vec<(String, String)>): keeps order and duplicates, case-insensitive get/set
  │       └── encoding.rs  // enum Encoding { Gzip } + with_gzip(handler): body compression
  ├── http1/               // HTTP/1.1 text codec (RFC 9112)
  │   ├── mod.rs           // read_request (head + body via Content-Length) and serve: keep-alive loop
  │   ├── parse.rs         // parse_request, parse_headers, content_length
  │   ├── serialize.rs     // serialize_response: status line, headers, Content-Length, Connection: close
  │   └── connection.rs    // Connection header (close / keep-alive) + wants_close
  ├── http2/               // HTTP/2 binary codec (RFC 9113), work in progress
  │   └── mod.rs           // PREFACE (24 bytes) + serve (stub)
  └── handlers/            // business logic, depends on http/ only
      ├── mod.rs           // handle(&Request, &Config) -> Response: routes on method + path
      ├── echo.rs          // GET /echo/{str}
      ├── user_agent.rs    // GET /user-agent
      └── files.rs         // GET and POST /files/{name}

tests/
  └── http_server.rs       // integration tests against the real server over TCP (HTTP/1.1 + h2 detection)
```

Dependency direction: `server` wires `http1` / `http2` and `handlers` together, and they all depend on `http`.
Never the other way around: handlers don't know which protocol carried the request.
