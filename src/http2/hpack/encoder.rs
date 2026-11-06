// RFC 7541 §6.2.2: the minimal encoder only emits "literal header field
// without indexing, new name", one per field:
//
//     0 0 0 0 │ 0 0 0 0      name string      value string
//     no idx  │ index 0 = the name is a literal, not a table index
//
// It never touches the dynamic table, so it has no state and cannot desync the peer.
// Names are encoded as given: lowercasing them is the HTTP/2 codec's job (RFC 9113 §8.2.1).
pub fn encode(fields: &[(&str, &str)]) -> Vec<u8> {
    todo!("step 5.3")
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http2::hpack::decoder::Decoder;

    const DEFAULT_LIMIT: usize = 4_096;
    const MAX_HEADER_LIST_SIZE: usize = 16_384;

    // Builds the expected bytes of one field: 0x00 + name literal + value literal.
    // Only for names and values shorter than 127 bytes (one-byte length).
    fn field(name: &str, value: &str) -> Vec<u8> {
        assert!(name.len() < 127 && value.len() < 127);
        let mut bytes = vec![0x00, name.len() as u8];
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(value.len() as u8);
        bytes.extend_from_slice(value.as_bytes());
        bytes
    }

    fn decoded(block: &[u8]) -> Vec<(String, String)> {
        Decoder::new(DEFAULT_LIMIT, MAX_HEADER_LIST_SIZE)
            .decode(block)
            .expect("the step 4 decoder rejected the block")
            .into_iter()
            .map(|(name, value)| {
                (
                    String::from_utf8(name).unwrap(),
                    String::from_utf8(value).unwrap(),
                )
            })
            .collect()
    }

    fn pairs(fields: &[(&str, &str)]) -> Vec<(String, String)> {
        fields
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    // --- Exact bytes ---

    #[test]
    fn encodes_nothing_for_an_empty_list() {
        assert_eq!(encode(&[]), Vec::<u8>::new());
    }

    // C.2.1 encodes the same field with 0x40 (incremental indexing).
    // Without indexing, only the first byte changes: 0x00.
    #[test]
    fn encodes_c_2_1_without_indexing() {
        let expected = [
            0x00, // literal without indexing, new name
            0x0a, b'c', b'u', b's', b't', b'o', b'm', b'-', b'k', b'e', b'y', // name
            0x0d, b'c', b'u', b's', b't', b'o', b'm', b'-', b'h', b'e', b'a', b'd', b'e',
            b'r', // value
        ];

        assert_eq!(encode(&[("custom-key", "custom-header")]), expected);
    }

    // 13 bytes, where the static table (index 8) would need one: 0x88.
    #[test]
    fn encodes_the_status_pseudo_header_as_a_literal() {
        assert_eq!(
            encode(&[(":status", "200")]),
            [
                0x00, 0x07, b':', b's', b't', b'a', b't', b'u', b's', 0x03, b'2', b'0', b'0'
            ]
        );
    }

    #[test]
    fn encodes_an_empty_value() {
        assert_eq!(encode(&[("x-empty", "")]), field("x-empty", ""));
    }

    // Fields are concatenated in order, nothing between them.
    #[test]
    fn concatenates_fields_in_order() {
        let mut expected = field(":status", "200");
        expected.extend(field("content-type", "text/plain"));
        expected.extend(field("content-length", "3"));

        assert_eq!(
            encode(&[
                (":status", "200"),
                ("content-type", "text/plain"),
                ("content-length", "3"),
            ]),
            expected
        );
    }

    // HPACK is case-agnostic: the encoder must not rewrite what it is given.
    #[test]
    fn encodes_names_as_given() {
        assert_eq!(encode(&[("X-Custom", "v")]), field("X-Custom", "v"));
    }

    // --- Round trip with the step 4 decoder ---

    #[test]
    fn round_trips_a_typical_response() {
        let fields = [
            (":status", "200"),
            ("content-type", "text/plain"),
            ("content-encoding", "gzip"),
            ("content-length", "23"),
        ];

        assert_eq!(decoded(&encode(&fields)), pairs(&fields));
    }

    // Duplicates are legal (e.g. several set-cookie): keep them all, in order.
    #[test]
    fn round_trips_duplicate_names() {
        let fields = [("set-cookie", "a=1"), ("set-cookie", "b=2")];

        assert_eq!(decoded(&encode(&fields)), pairs(&fields));
    }

    // Lengths above 127 need continuation bytes, in names and values alike.
    #[test]
    fn round_trips_long_names_and_values() {
        let name = "x-".to_string() + &"n".repeat(200);
        let value = "v".repeat(1_000);
        let fields = [(name.as_str(), value.as_str())];

        assert_eq!(decoded(&encode(&fields)), pairs(&fields));
    }

    // No dynamic table: every block decodes on its own, with a fresh decoder,
    // and a decoder whose table is disabled (size 0) reads it just as well.
    #[test]
    fn blocks_are_independent_of_any_table() {
        let fields = [(":status", "404"), ("content-length", "0")];
        let block = encode(&fields);

        let mut no_table = Decoder::new(0, MAX_HEADER_LIST_SIZE);
        let first = no_table.decode(&block).unwrap();
        let second = no_table.decode(&block).unwrap();

        assert_eq!(first, second);
        assert_eq!(decoded(&block), pairs(&fields));
    }
}
