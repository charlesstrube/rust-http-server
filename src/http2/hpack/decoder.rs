use crate::http2::hpack::{
    HeaderField, HpackError, dynamic_table::DynamicTable, integer::decode_integer,
    representation::Representation, static_table, string::decode_string,
};

const STATIC_TABLE_LEN: usize = 61;
const MAX_HEADER_LIST_SIZE: usize = 16_384;
// RFC 7541 §3 and §6: turns header blocks into (name, value) pairs.
// One decoder per connection: the dynamic table is shared by every block, in order.
pub struct Decoder {
    table: DynamicTable,
    max_list_size: usize,
}

impl Decoder {
    pub fn new(limit: usize, max_list_size: usize) -> Self {
        Self {
            table: DynamicTable::new(limit),
            max_list_size,
        }
    }

    fn resolve(&self, index_id: usize) -> Result<HeaderField, HpackError> {
        if index_id <= STATIC_TABLE_LEN {
            let (name, value) = static_table::get(index_id)?;
            Ok((name.as_bytes().to_vec(), value.as_bytes().to_vec()))
        } else {
            let index = index_id - STATIC_TABLE_LEN;
            let (name, value) = self.table.get(index)?;
            Ok((name.to_vec(), value.to_vec()))
        }
    }

    fn push(
        &self,
        list: &mut Vec<HeaderField>,
        list_size: &mut usize,
        field: HeaderField,
    ) -> Result<(), HpackError> {
        *list_size += field.0.len() + field.1.len() + 32;
        if *list_size > self.max_list_size {
            return Err(HpackError::HeaderListTooLarge);
        }
        list.push(field);
        Ok(())
    }

    fn read_literal(
        &self,
        block: &[u8],
        prefix_bits: u8,
    ) -> Result<(HeaderField, usize), HpackError> {
        let mut cursor = 0;
        let (index_id, consumed) = decode_integer(block, prefix_bits)?;
        cursor += consumed;
        let name = if index_id == 0 {
            let (name, consumed) = decode_string(&block[cursor..])?;
            cursor += consumed;
            name
        } else {
            self.resolve(index_id)?.0
        };
        let (value, consumed) = decode_string(&block[(cursor)..])?;
        cursor += consumed;

        Ok(((name, value), cursor))
    }

    // Decodes one complete header block: the HEADERS payload, plus any
    // CONTINUATION payloads, concatenated.
    pub fn decode(&mut self, block: &[u8]) -> Result<Vec<HeaderField>, HpackError> {
        let mut cursor = 0;

        let mut list = vec![];
        let mut list_size = 0usize;
        while cursor < block.len() {
            let representation = Representation::from_byte(block[cursor]);
            let prefix_bits = representation.prefix_bits();

            match representation {
                Representation::LiteralIndexing => {
                    let ((name, value), consumed) =
                        self.read_literal(&block[cursor..], prefix_bits)?;
                    cursor += consumed;
                    self.push(&mut list, &mut list_size, (name.clone(), value.clone()))?;
                    self.table.insert(name, value);
                }
                Representation::SizeUpdate => {
                    if !list.is_empty() {
                        return Err(HpackError::UnexpectedTableSizeUpdate);
                    }
                    let (value, consumed) = decode_integer(&block[cursor..], prefix_bits)?;
                    cursor += consumed;
                    self.table.set_max_size(value)?;
                }
                Representation::Indexed => {
                    let (index_id, consumed) = decode_integer(&block[cursor..], prefix_bits)?;
                    cursor += consumed;
                    self.push(&mut list, &mut list_size, self.resolve(index_id)?)?;
                }
                Representation::NoIndexing | Representation::NeverIndexed => {
                    let ((name, value), consumed) =
                        self.read_literal(&block[cursor..], prefix_bits)?;
                    cursor += consumed;
                    self.push(&mut list, &mut list_size, (name, value))?;
                }
            }
        }

        Ok(list)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const DEFAULT_LIMIT: usize = 4_096;

    // Decodes with `decoder` and turns the headers into strings for readable asserts.
    fn decode_with(
        decoder: &mut Decoder,
        block: &[u8],
    ) -> Result<Vec<(String, String)>, HpackError> {
        let headers = decoder.decode(block)?;
        Ok(headers
            .into_iter()
            .map(|(name, value)| {
                (
                    String::from_utf8(name).unwrap(),
                    String::from_utf8(value).unwrap(),
                )
            })
            .collect())
    }

    fn decode(block: &[u8]) -> Result<Vec<(String, String)>, HpackError> {
        decode_with(
            &mut Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE),
            block,
        )
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    // --- Indexed header field (RFC 7541 §6.1): 1xxxxxxx ---

    // C.2.4
    #[test]
    fn indexed_field_from_the_static_table() {
        assert_eq!(decode(&[0x82]), Ok(pairs(&[(":method", "GET")])));
    }

    #[test]
    fn indexed_field_with_an_empty_value() {
        assert_eq!(decode(&[0x81]), Ok(pairs(&[(":authority", "")])));
    }

    // Headers come out in the order they were encoded.
    #[test]
    fn several_indexed_fields_keep_their_order() {
        assert_eq!(
            decode(&[0x82, 0x86, 0x84]),
            Ok(pairs(&[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/")
            ]))
        );
    }

    // RFC 7541 §6.1: index 0 is not used.
    #[test]
    fn indexed_field_0_is_invalid() {
        assert_eq!(decode(&[0x80]), Err(HpackError::InvalidIndex));
    }

    #[test]
    fn empty_block_has_no_header() {
        assert_eq!(decode(&[]), Ok(vec![]));
    }

    // --- Literal without indexing (RFC 7541 §6.2.2): 0000xxxx ---

    // C.2.2: name from static index 4 (:path), literal value.
    #[test]
    fn literal_without_indexing_with_an_indexed_name() {
        let block = [&[0x04, 0x0c][..], b"/sample/path"].concat();

        assert_eq!(decode(&block), Ok(pairs(&[(":path", "/sample/path")])));
    }

    // Name index 0: the name is a string literal too.
    #[test]
    fn literal_without_indexing_with_a_literal_name() {
        let block = [&[0x00, 0x03][..], b"foo", &[0x03], b"bar"].concat();

        assert_eq!(decode(&block), Ok(pairs(&[("foo", "bar")])));
    }

    // 4-bit prefix: index 58 (user-agent) = 15 + 43, so it needs a continuation byte.
    #[test]
    fn literal_name_index_that_overflows_the_4_bit_prefix() {
        let block = [&[0x0f, 0x2b, 0x03][..], b"abc"].concat();

        assert_eq!(decode(&block), Ok(pairs(&[("user-agent", "abc")])));
    }

    // --- Literal never indexed (RFC 7541 §6.2.3): 0001xxxx ---

    // C.2.3: decoded exactly like "without indexing"; the flag only matters to proxies.
    #[test]
    fn literal_never_indexed_with_a_literal_name() {
        let block = [&[0x10, 0x08][..], b"password", &[0x06], b"secret"].concat();

        assert_eq!(decode(&block), Ok(pairs(&[("password", "secret")])));
    }

    #[test]
    fn literal_never_indexed_with_an_indexed_name() {
        let block = [&[0x14, 0x02][..], b"/a"].concat();

        assert_eq!(decode(&block), Ok(pairs(&[(":path", "/a")])));
    }

    // --- Mixing representations ---

    #[test]
    fn indexed_and_literal_fields_in_one_block() {
        let block = [
            &[0x82, 0x04, 0x02][..],
            b"/a",
            &[0x00, 0x01],
            b"x",
            &[0x01],
            b"y",
        ]
        .concat();

        assert_eq!(
            decode(&block),
            Ok(pairs(&[(":method", "GET"), (":path", "/a"), ("x", "y")]))
        );
    }

    // --- Errors ---

    // The value string is missing entirely.
    #[test]
    fn literal_without_its_value_is_truncated() {
        assert_eq!(decode(&[0x04]), Err(HpackError::InvalidTruncated));
    }

    // The value announces 5 bytes but only 1 follows.
    #[test]
    fn literal_with_a_short_value_is_truncated() {
        assert_eq!(
            decode(&[0x04, 0x05, b'/']),
            Err(HpackError::InvalidTruncated)
        );
    }

    // A truncated field after a valid one fails the whole block.
    #[test]
    fn error_after_a_valid_field_fails_the_block() {
        assert_eq!(decode(&[0x82, 0x04]), Err(HpackError::InvalidTruncated));
    }

    // A Huffman error inside a field fails the whole block.
    #[test]
    fn invalid_huffman_value_fails_the_block() {
        assert_eq!(
            decode(&[0x04, 0x81, 0x00]),
            Err(HpackError::InvalidHuffmanPadding)
        );
    }

    // --- Needs the dynamic table (4.5) ---

    // C.2.1: the header is returned AND added to the dynamic table.
    #[test]
    fn literal_with_indexing_is_added_to_the_table() {
        let block = [&[0x40, 0x0a][..], b"custom-key", &[0x0d], b"custom-header"].concat();
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);

        assert_eq!(
            decode_with(&mut decoder, &block),
            Ok(pairs(&[("custom-key", "custom-header")]))
        );
        assert_eq!(decoder.table.len(), 1);
        assert_eq!(decoder.table.size(), 55);
    }

    #[test]
    fn literal_without_indexing_is_not_added_to_the_table() {
        let block = [&[0x04, 0x0c][..], b"/sample/path"].concat();
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);
        decoder.decode(&block).unwrap();

        assert_eq!(decoder.table.len(), 0);
    }

    #[test]
    fn literal_never_indexed_is_not_added_to_the_table() {
        let block = [&[0x10, 0x08][..], b"password", &[0x06], b"secret"].concat();
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);
        decoder.decode(&block).unwrap();

        assert_eq!(decoder.table.len(), 0);
    }

    // Index 62 is the first dynamic entry: invalid while the table is empty.
    #[test]
    fn index_past_the_static_table_with_an_empty_dynamic_table() {
        assert_eq!(decode(&[0xbe]), Err(HpackError::InvalidIndex));
    }

    // RFC 7541 Appendix C.3: three requests on the same connection.
    const C_3_1: &[u8] = &[
        0x82, 0x86, 0x84, 0x41, 0x0f, b'w', b'w', b'w', b'.', b'e', b'x', b'a', b'm', b'p', b'l',
        b'e', b'.', b'c', b'o', b'm',
    ];
    const C_3_2: &[u8] = &[
        0x82, 0x86, 0x84, 0xbe, 0x58, 0x08, b'n', b'o', b'-', b'c', b'a', b'c', b'h', b'e',
    ];
    const C_3_3: &[u8] = &[
        0x82, 0x87, 0x85, 0xbf, 0x40, 0x0a, b'c', b'u', b's', b't', b'o', b'm', b'-', b'k', b'e',
        b'y', 0x0c, b'c', b'u', b's', b't', b'o', b'm', b'-', b'v', b'a', b'l', b'u', b'e',
    ];

    #[test]
    fn c_3_1_first_request() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);

        assert_eq!(
            decode_with(&mut decoder, C_3_1),
            Ok(pairs(&[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/"),
                (":authority", "www.example.com"),
            ]))
        );
        assert_eq!(decoder.table.size(), 57);
    }

    // 0xbe = index 62: :authority, learned from the first request.
    #[test]
    fn c_3_2_reuses_the_first_request() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);
        decoder.decode(C_3_1).unwrap();

        assert_eq!(
            decode_with(&mut decoder, C_3_2),
            Ok(pairs(&[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/"),
                (":authority", "www.example.com"),
                ("cache-control", "no-cache"),
            ]))
        );
        assert_eq!(decoder.table.size(), 110);
    }

    // 0xbf = index 63: :authority has shifted up since cache-control was added.
    #[test]
    fn c_3_3_indices_shift_as_entries_are_added() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);
        decoder.decode(C_3_1).unwrap();
        decoder.decode(C_3_2).unwrap();

        assert_eq!(
            decode_with(&mut decoder, C_3_3),
            Ok(pairs(&[
                (":method", "GET"),
                (":scheme", "https"),
                (":path", "/index.html"),
                (":authority", "www.example.com"),
                ("custom-key", "custom-value"),
            ]))
        );
        assert_eq!(decoder.table.size(), 164);
    }

    // 0x7e = literal with indexing, name index 62 (:authority from C.3.1).
    #[test]
    fn literal_name_from_the_dynamic_table() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);
        decoder.decode(C_3_1).unwrap();
        let block = [&[0x7e, 0x03][..], b"foo"].concat();

        assert_eq!(
            decode_with(&mut decoder, &block),
            Ok(pairs(&[(":authority", "foo")]))
        );
        assert_eq!(decoder.table.len(), 2);
    }

    // --- Dynamic table size update (RFC 7541 §6.3): 001xxxxx ---

    // 0x3f 0xe1 0x1f = 31 + 97 + 31 * 128 = 4_096. A size update produces no header.
    #[test]
    fn size_update_at_the_start_of_a_block() {
        assert_eq!(
            decode(&[0x3f, 0xe1, 0x1f, 0x82]),
            Ok(pairs(&[(":method", "GET")]))
        );
    }

    #[test]
    fn size_update_to_0_empties_the_table() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);
        decoder.decode(C_3_1).unwrap();
        decoder.decode(&[0x20]).unwrap();

        assert_eq!(decoder.table.len(), 0);
    }

    // RFC 7541 §4.2: up to two updates may open a block (shrink, then set the new size).
    #[test]
    fn two_size_updates_at_the_start_are_allowed() {
        assert_eq!(
            decode(&[0x20, 0x3f, 0xe1, 0x1f, 0x82]),
            Ok(pairs(&[(":method", "GET")]))
        );
    }

    // "Start" means the start of each block, not of the connection.
    #[test]
    fn size_update_at_the_start_of_a_later_block() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);
        decoder.decode(&[0x82]).unwrap();

        assert_eq!(
            decode_with(&mut decoder, &[0x20, 0x82]),
            Ok(pairs(&[(":method", "GET")]))
        );
    }

    #[test]
    fn size_update_after_a_header_is_rejected() {
        assert_eq!(
            decode(&[0x82, 0x20]),
            Err(HpackError::UnexpectedTableSizeUpdate)
        );
    }

    // 0x3f 0xe2 0x1f = 4_097: above the SETTINGS_HEADER_TABLE_SIZE we announced.
    #[test]
    fn size_update_above_the_limit_is_rejected() {
        assert_eq!(
            decode(&[0x3f, 0xe2, 0x1f]),
            Err(HpackError::InvalidTableSize)
        );
    }

    // --- RFC 7541 Appendix C.4: C.3 with Huffman (4.6) ---

    // The RFC hex dumps, spaces ignored.
    fn hex(dump: &str) -> Vec<u8> {
        let digits: Vec<char> = dump.chars().filter(|c| !c.is_whitespace()).collect();
        digits
            .chunks(2)
            .map(|pair| u8::from_str_radix(&pair.iter().collect::<String>(), 16).unwrap())
            .collect()
    }

    const C_4_1: &str = "8286 8441 8cf1 e3c2 e5f2 3a6b a0ab 90f4 ff";
    const C_4_2: &str = "8286 84be 5886 a8eb 1064 9cbf";
    const C_4_3: &str = "8287 85bf 4088 25a8 49e9 5ba9 7d7f 8925 a849 e95b b8e8 b4bf";

    // The table stores decoded values: same sizes as C.3.
    #[test]
    fn c_4_huffman_requests() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);

        assert_eq!(
            decode_with(&mut decoder, &hex(C_4_1)),
            Ok(pairs(&[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/"),
                (":authority", "www.example.com"),
            ]))
        );
        assert_eq!(decoder.table.size(), 57);

        assert_eq!(
            decode_with(&mut decoder, &hex(C_4_2)),
            Ok(pairs(&[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/"),
                (":authority", "www.example.com"),
                ("cache-control", "no-cache"),
            ]))
        );
        assert_eq!(decoder.table.size(), 110);

        assert_eq!(
            decode_with(&mut decoder, &hex(C_4_3)),
            Ok(pairs(&[
                (":method", "GET"),
                (":scheme", "https"),
                (":path", "/index.html"),
                (":authority", "www.example.com"),
                ("custom-key", "custom-value"),
            ]))
        );
        assert_eq!(decoder.table.size(), 164);
    }

    // --- RFC 7541 Appendix C.6: responses with Huffman and evictions ---

    const C_6_1: &str = "4882 6402 5885 aec3 771a 4b61 96d0 7abe 9410 54d4 44a8 2005 9504 0b81 \
                         66e0 82a6 2d1b ff6e 919d 29ad 1718 63c7 8f0b 97c8 e9ae 82ae 43d3";
    const C_6_2: &str = "4883 640e ffc1 c0bf";
    const C_6_3: &str = "88c1 6196 d07a be94 1054 d444 a820 0595 040b 8166 e084 a62d 1bff c05a \
                         839b d9ab 77ad 94e7 821d d7f2 e6c7 b335 dfdf cd5b 3960 d5af 2708 7f36 \
                         72c1 ab27 0fb5 291f 9587 3160 65c0 03ed 4ee5 b106 3d50 07";

    // SETTINGS_HEADER_TABLE_SIZE = 256: each response evicts older entries.
    #[test]
    fn c_6_huffman_responses_with_evictions() {
        let mut decoder = Decoder::new(256, MAX_HEADER_LIST_SIZE);

        assert_eq!(
            decode_with(&mut decoder, &hex(C_6_1)),
            Ok(pairs(&[
                (":status", "302"),
                ("cache-control", "private"),
                ("date", "Mon, 21 Oct 2013 20:13:21 GMT"),
                ("location", "https://www.example.com"),
            ]))
        );
        assert_eq!(decoder.table.size(), 222);

        // ":status: 302" (42 bytes) is evicted to make room for ":status: 307".
        assert_eq!(
            decode_with(&mut decoder, &hex(C_6_2)),
            Ok(pairs(&[
                (":status", "307"),
                ("cache-control", "private"),
                ("date", "Mon, 21 Oct 2013 20:13:21 GMT"),
                ("location", "https://www.example.com"),
            ]))
        );
        assert_eq!(decoder.table.size(), 222);

        assert_eq!(
            decode_with(&mut decoder, &hex(C_6_3)),
            Ok(pairs(&[
                (":status", "200"),
                ("cache-control", "private"),
                ("date", "Mon, 21 Oct 2013 20:13:22 GMT"),
                ("location", "https://www.example.com"),
                ("content-encoding", "gzip"),
                (
                    "set-cookie",
                    "foo=ASDJKHQKBZXOQWEOPIUAXQWEOIU; max-age=3600; version=1",
                ),
            ]))
        );
        assert_eq!(decoder.table.size(), 215);
        assert_eq!(decoder.table.len(), 3);
    }

    // --- Header list size (RFC 9113 §6.5.2, SETTINGS_MAX_HEADER_LIST_SIZE) ---

    fn decode_limited(max_list_size: usize, block: &[u8]) -> Result<Vec<HeaderField>, HpackError> {
        Decoder::new(DEFAULT_LIMIT, max_list_size).decode(block)
    }

    // ":method: GET" = 7 + 3 + 32 = 42: the same overhead as a table entry.
    // The limit itself is allowed, one byte less is not.
    #[test]
    fn field_size_counts_the_32_byte_overhead() {
        assert!(decode_limited(42, &[0x82]).is_ok());
        assert_eq!(
            decode_limited(41, &[0x82]),
            Err(HpackError::HeaderListTooLarge)
        );
    }

    // The limit applies to the sum of the fields, not to each one.
    #[test]
    fn limit_applies_to_the_whole_list() {
        assert!(decode_limited(84, &[0x82, 0x82]).is_ok());
        assert_eq!(
            decode_limited(84, &[0x82, 0x82, 0x82]),
            Err(HpackError::HeaderListTooLarge)
        );
    }

    // Each representation is counted: "foo: bar" = 3 + 3 + 32 = 38.
    #[test]
    fn literal_fields_are_counted() {
        let without_indexing = [&[0x00, 0x03][..], b"foo", &[0x03], b"bar"].concat();
        let never_indexed = [&[0x10, 0x03][..], b"foo", &[0x03], b"bar"].concat();
        let with_indexing = [&[0x40, 0x03][..], b"foo", &[0x03], b"bar"].concat();

        for block in [without_indexing, never_indexed, with_indexing] {
            assert!(decode_limited(38, &block).is_ok());
            assert_eq!(
                decode_limited(37, &block),
                Err(HpackError::HeaderListTooLarge)
            );
        }
    }

    // The size is the decoded one: ":authority: www.example.com" is 10 + 15 + 32 = 57,
    // even though the Huffman value takes only 12 bytes on the wire.
    #[test]
    fn huffman_fields_count_their_decoded_size() {
        let block = hex("418c f1e3 c2e5 f23a 6ba0 ab90 f4ff");

        assert!(decode_limited(57, &block).is_ok());
        assert_eq!(
            decode_limited(56, &block),
            Err(HpackError::HeaderListTooLarge)
        );
    }

    // A size update produces no field: it costs nothing.
    // 0x3f e1 1f = size update to 4_096 (31 + 97 + 31 * 128).
    #[test]
    fn size_updates_are_not_counted() {
        assert!(decode_limited(42, &[0x3f, 0xe1, 0x1f, 0x82]).is_ok());
    }

    // The limit is per header block: the count starts over at each decode.
    #[test]
    fn count_starts_over_for_each_block() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, 84);

        assert!(decoder.decode(&[0x82, 0x82]).is_ok());
        assert!(decoder.decode(&[0x82, 0x82]).is_ok());
    }

    // The HPACK bomb: one ~4 KB entry in the dynamic table, then a block of
    // 1-byte references to it (0xbe = index 62). Without a limit, 1_000 bytes
    // on the wire would decode to ~4 MB.
    #[test]
    fn hpack_bomb_is_rejected() {
        let mut decoder = Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE);

        // Literal with indexing, name "x", value of 4_000 bytes:
        // length 4_000 = 127 + 33 + 30 * 128 → 0x7f 0xa1 0x1e.
        let mut insert = vec![0x40, 0x01, b'x', 0x7f, 0xa1, 0x1e];
        insert.extend_from_slice(&[b'a'; 4_000]);
        assert!(decoder.decode(&insert).is_ok());
        assert_eq!(decoder.table.size(), 1 + 4_000 + 32);

        assert_eq!(
            decoder.decode(&[0xbe; 1_000]),
            Err(HpackError::HeaderListTooLarge)
        );
    }
}
