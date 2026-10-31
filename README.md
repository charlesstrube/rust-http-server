# Rust HTTP/1.1 and HTTP/2 protocal trainging

HTTP/1.1 works

I'm doing HTTP/2

I won't do HTTP/3 but might pick this up later

## What travels over HTTP/2, the big picture

### The picture: a pipe and envelopes

In **HTTP/1.1**, the TCP pipe carries a continuous stream of text: one request, its response, the next one, each in turn.

In **HTTP/2**, the same TCP pipe carries a sequence of **small envelopes**, the *frames*. Each envelope carries:

- **its size**;
- **its type**: what it contains;
- **a conversation number**: the *stream*.

What is inside the envelope depends on its type.

### Two families of envelopes

**1. Connection housekeeping**: number 0, "this is about the pipe itself".

| Envelope | Roughly |
|---|---|
| SETTINGS | "Here are my rules: max envelope size, how many conversations in parallel…" |
| SETTINGS ACK | "Got your rules." |
| PING / PING ACK | "Are you still there?" / "Yes." |
| WINDOW_UPDATE | "You can send me X more bytes." |
| GOAWAY | "I'm hanging up." |

**2. Requests and responses**: numbers 1, 3, 5… One number = one request and its response.

| Envelope | Roughly | HTTP/1.1 equivalent |
|---|---|---|
| HEADERS | The "who / what / where": method, path, headers, status | Request line or status line, plus the headers |
| DATA | The content | The body |

The **END_STREAM** flag on the last envelope of a number means: "I'm done talking on this conversation".

### An exchange with nghttp, simplified

```
Client → "Hello, I speak HTTP/2"                         (preface)
Client → "My rules: …"                                   (SETTINGS, #0)
Client → "Conversation #1: GET /echo/abc, that's all"    (HEADERS, #1, END_STREAM)
Server → "My rules: the defaults"                        (SETTINGS, #0)
Server → "Got your rules"                                (SETTINGS ACK, #0)
Client → "Got yours"                                     (SETTINGS ACK, #0)
           … the client waits for the response to #1 …
Server → "Conversation #1: 200 OK"                       (HEADERS, #1)
Server → "Conversation #1: abc, that's all"              (DATA, #1, END_STREAM)
```

### The two key ideas

1. **The conversation number is what enables parallelism.** Envelopes for #1 and #3 can arrive interleaved in the pipe; each side sorts them back by number. In HTTP/1.1, nothing said which request a piece of text belonged to, hence "one at a time".
2. **The content of a HEADERS frame is compressed (HPACK).** Instead of spelling out `user-agent: nghttp2/1.70.0`, the sender often just says "entry #62 of the dictionary we share".

### To observe it

```sh
./your_program.sh
nghttp -nv http://localhost:4221/echo/abc
```

`nghttp -nv` prints every frame sent and received.
