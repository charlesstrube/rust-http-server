use crate::http2::hpack::{
    HpackError, dynamic_table::DynamicTable, integer::decode_integer, static_table::get,
    string::decode_string,
};

/*
0x80..=0xFF     1xxx xxxx   indexed
0x40..=0x7F	    01xx xxxx	literal + indexing
0x20..=0x3F	    001x xxxx	size update
0x10..=0x1F	    0001 xxxx	never indexed
0x00..=0x0F	    0000 xxxx	no indexing
*/
enum DecoderDecision {
    Indexed,
    LiteralIndexing,
    SizeUpdate,
    NeverIndexed,
    NoIndexing,
}

// RFC 7541 §3 and §6: turns header blocks into (name, value) pairs.
// One decoder per connection: the dynamic table is shared by every block, in order.
pub struct Decoder {
    table: DynamicTable,
}

fn get_decision(byte: &u8) -> DecoderDecision {
    if byte & 0x80 == 0b1000_0000 {
        println!("DecoderDecision::Indexed");
        DecoderDecision::Indexed
    } else if byte & 0xC0 == 0b0100_0000 {
        println!("DecoderDecision::LiteralIndexing");
        DecoderDecision::LiteralIndexing
    } else if byte & 0xF0 == 0b0000_0000 {
        println!("DecoderDecision::NoIndexing");
        DecoderDecision::NoIndexing
    } else if byte & 0xF0 == 0b0001_0000 {
        println!("DecoderDecision::NeverIndexed");
        DecoderDecision::NeverIndexed
    } else {
        println!("DecoderDecision::SizeUpdate");
        // 0010_0000
        DecoderDecision::SizeUpdate
    }
}

impl Decoder {
    // `limit` is the SETTINGS_HEADER_TABLE_SIZE we announced (4_096 by default).
    pub fn new(limit: usize) -> Self {
        Self {
            table: DynamicTable::new(limit),
        }
    }

    // Decodes one complete header block: the HEADERS payload, plus any
    // CONTINUATION payloads, concatenated.
    pub fn decode(&mut self, block: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, HpackError> {
        let mut pos = 0;

        let mut list = vec![];
        while pos < block.len() {
            let mut cursor = pos;
            let byte = block[pos];

            let decode_interger_and_update = |number: u8| -> Result<usize, HpackError> {
                let (number, consumed) = decode_integer(&block[cursor..], number)?;
                cursor += consumed;
                Ok(number)
            };

            let decode_string_and_update = || -> Result<Vec<u8>, HpackError> {
                let (value, consumed) = decode_string(&block[cursor..])?;
                cursor += consumed;
                Ok(value)
            };

            match get_decision(&byte) {
                DecoderDecision::LiteralIndexing => {
                    let (index_id, consumed) = decode_integer(&block[cursor..], 6)?;
                    cursor += consumed;

                    let name = if index_id == 0 {
                        let (name, consumed) = decode_string(&block[cursor..])?;
                        cursor += consumed;
                        name
                    } else if index_id < 62 {
                        let (name, _) = get(index_id)?;
                        name.as_bytes().to_vec()
                    } else {
                        let index = index_id.checked_sub(61).ok_or(HpackError::InvalidIndex)?;
                        let (name, _) = self.table.get(index)?;
                        name.to_vec()
                    };

                    let (value, consumed) = decode_string(&block[(cursor)..])?;
                    cursor += consumed;

                    list.push((name.clone(), value.clone()));
                    self.table.insert(name, value);
                }
                DecoderDecision::SizeUpdate => {
                    if list.len() > 0 {
                        return Err(HpackError::UnexpectedTableSizeUpdate);
                    }
                    let (value, consumed) = decode_integer(&block[cursor..], 5)?;
                    cursor += consumed;
                    self.table.set_max_size(value)?;
                }
                DecoderDecision::Indexed => {
                    let (index_id, consumed) = decode_integer(&block[cursor..], 7)?;
                    cursor += consumed;
                    if index_id < 62 {
                        let (name, value) = get(index_id)?;
                        list.push((name.as_bytes().to_vec(), value.as_bytes().to_vec()));
                    } else {
                        let index = index_id.checked_sub(61).ok_or(HpackError::InvalidIndex)?;
                        let (name, value) = self.table.get(index)?;
                        list.push((name.to_vec(), value.to_vec()));
                    }
                }
                DecoderDecision::NoIndexing | DecoderDecision::NeverIndexed => {
                    let (index_id, consumed) = decode_integer(&block[cursor..], 4)?;
                    cursor += consumed;
                    let name = if index_id == 0 {
                        let (name, consumed) = decode_string(&block[cursor..])?;
                        cursor += consumed;
                        name
                    } else if index_id < 62 {
                        let (name, _) = get(index_id)?;
                        name.as_bytes().to_vec()
                    } else {
                        let index = index_id.checked_sub(61).ok_or(HpackError::InvalidIndex)?;
                        let (name, _) = self.table.get(index)?;
                        name.to_vec()
                    };
                    let (value, consumed) = decode_string(&block[(cursor)..])?;
                    cursor += consumed;

                    list.push((name.clone(), value.clone()));
                }
            }
            pos = cursor;
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
        decode_with(&mut Decoder::new(DEFAULT_LIMIT), block)
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
        assert_eq!(decode(&[0x04]), Err(HpackError::Truncated));
    }

    // The value announces 5 bytes but only 1 follows.
    #[test]
    fn literal_with_a_short_value_is_truncated() {
        assert_eq!(decode(&[0x04, 0x05, b'/']), Err(HpackError::Truncated));
    }

    // A truncated field after a valid one fails the whole block.
    #[test]
    fn error_after_a_valid_field_fails_the_block() {
        assert_eq!(decode(&[0x82, 0x04]), Err(HpackError::Truncated));
    }

    // Until 4.6, a Huffman-encoded value must surface the string error.
    #[test]
    fn huffman_value_is_not_supported_yet() {
        assert_eq!(
            decode(&[0x04, 0x81, 0x00]),
            Err(HpackError::HuffmanUnsupported)
        );
    }

    // --- Needs the dynamic table (4.5) ---

    // C.2.1: the header is returned AND added to the dynamic table.
    #[test]
    fn literal_with_indexing_is_added_to_the_table() {
        let block = [&[0x40, 0x0a][..], b"custom-key", &[0x0d], b"custom-header"].concat();
        let mut decoder = Decoder::new(DEFAULT_LIMIT);

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
        let mut decoder = Decoder::new(DEFAULT_LIMIT);
        decoder.decode(&block).unwrap();

        assert_eq!(decoder.table.len(), 0);
    }

    #[test]
    fn literal_never_indexed_is_not_added_to_the_table() {
        let block = [&[0x10, 0x08][..], b"password", &[0x06], b"secret"].concat();
        let mut decoder = Decoder::new(DEFAULT_LIMIT);
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
        let mut decoder = Decoder::new(DEFAULT_LIMIT);

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
        let mut decoder = Decoder::new(DEFAULT_LIMIT);
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
        let mut decoder = Decoder::new(DEFAULT_LIMIT);
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
        let mut decoder = Decoder::new(DEFAULT_LIMIT);
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
        let mut decoder = Decoder::new(DEFAULT_LIMIT);
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
        let mut decoder = Decoder::new(DEFAULT_LIMIT);
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
}
