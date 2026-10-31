# Step 4: HPACK decoder

RFC 7541. The biggest step: split it into sub-steps, each with its own tests in `src/http2/hpack/`.

1. **Prefixed integers** (§5.1). An integer starts in the low N bits of a byte (N = 4 to 7, the high bits hold flags); if it does not fit, continuation bytes follow, 7 bits each, least significant first. Tests: examples C.1.1 to C.1.3 (10 on 5 bits, 1337 on 5 bits, 42 on 8 bits). Reject an integer that overflows.
2. **String literals** (§5.2). 1 bit H (Huffman or not) + length as a 7-bit prefixed integer + the bytes. Start without Huffman.
3. **Static table** (Appendix A). 61 fixed entries, index 1 = `:authority`, 2 = `:method GET`… Index 0 is an error.
4. **The four representations** (§6), identified by their first bits:
   - `1xxxxxxx`: indexed field (7-bit index);
   - `01xxxxxx`: literal **with** incremental indexing (6-bit index for the name, 0 = literal name), added to the dynamic table;
   - `0000xxxx`: literal without indexing (4-bit prefix);
   - `0001xxxx`: literal never indexed (4-bit prefix, same decoding, only matters to proxies);
   - `001xxxxx`: dynamic table size update (5-bit prefix).
5. **Dynamic table** (§2.3.2, §4). Indices continue after the static table: 62 = most recent entry. Entry size = `name.len() + value.len() + 32`. Inserting evicts the oldest entries until it fits; an entry larger than the whole table empties it. A size update above `SETTINGS_HEADER_TABLE_SIZE` (4 096) is an error.
6. **Huffman** (§5.2, Appendix B). Static code, 5 to 30 bits per symbol. Padding is at most 7 bits, all set to 1; longer padding or an EOS symbol in the data is an error. curl and nghttp always use it: step 4 is not done without it.

The table state is shared by every HEADERS block of the connection, in order: the decoder lives for the whole connection, not per request. Any decoding error is a **connection error** `COMPRESSION_ERROR` (§4.3 of RFC 9113): the two tables are out of sync, nothing can be decoded anymore.

Tests: **Appendix C** gives complete sequences (C.3 without Huffman, C.4 with, C.5 and C.6 with evictions) with the expected headers *and* the expected dynamic table after each request. A ready-made test suite.

[← Roadmap](README.md)
