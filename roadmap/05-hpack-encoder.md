# Step 5: HPACK encoder (minimal)

An encoder that only emits **literal without indexing, literal name, no Huffman** (`0000 0000` + name + value) is fully valid: it never touches the dynamic table, so it cannot desync the client.

- Names must be lowercase (RFC 9113 §8.2.1).
- Optional optimizations, later: use the static table (`:status 200` is index 8, a single byte), then Huffman, then the dynamic table.

Tests: encode then decode with your step 4 decoder; compare with `nghttp -nv` output.

[← Roadmap](README.md)
