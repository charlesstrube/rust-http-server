use crate::http2::hpack::{HpackError, integer::decode_integer};

const HUFFMAN_FLAG: u8 = 0x80;
const LENGTH_PREFIX_BITS: u8 = 7;

// RFC 7541 §5.2: decodes a string literal (H bit + 7-bit prefixed length + data).
// Returns the raw bytes and the number of bytes consumed.
//
// The first byte is read twice: bit 7 is the H flag, its 7 low bits start the length.
//
//     0 0 0 0 1 1 1 1 │ w w w . e x a m p l e . c o m │ 0x82 ...
//     H └─ length ──┘ │ └──────────── data ─────────┘ │ next field, not consumed
//     ▲               ▲                               ▲
//     0               consumed = 1                    end = 16
//
//     decode_integer(input, 7)  → (length = 15, consumed = 1)
//     end = consumed + length   → data = input[1..16]  → (b"www.example.com", 16)
//
// When the length overflows its prefix (≥ 127), decode_integer consumes more
// bytes and the data starts later: always slice from `consumed`, never from 1.
// With H = 1 the data is Huffman-coded (sub-step 4.6).
pub fn decode_string(input: &[u8]) -> Result<(Vec<u8>, usize), HpackError> {
    let first = input.first().ok_or(HpackError::Truncated)?;

    // if the first bytes is 1, then it's not supported
    let huffman = first & HUFFMAN_FLAG != 0;

    let (length, consumed) = decode_integer(input, LENGTH_PREFIX_BITS)?;

    if huffman {
        return Err(HpackError::HuffmanUnsupported);
    }

    let end = consumed + length;

    let data = input.get(consumed..end).ok_or(HpackError::Truncated)?;

    Ok((data.to_vec(), end))
}

#[cfg(test)]
mod test {
    use super::*;

    // Builds a non-Huffman string literal whose length fits in the 7-bit prefix.
    fn literal(data: &[u8]) -> Vec<u8> {
        assert!(
            data.len() < 127,
            "use an explicit encoding for longer strings"
        );
        let mut bytes = vec![data.len() as u8];
        bytes.extend_from_slice(data);
        bytes
    }

    // --- RFC 7541 Appendix C ---

    // C.3.1: the :authority value, 0x0f = H 0 + length 15.
    #[test]
    fn decodes_the_authority_of_c_3_1() {
        assert_eq!(
            decode_string(&literal(b"www.example.com")),
            Ok((b"www.example.com".to_vec(), 16))
        );
    }

    // C.2.1: a literal header name, 0x0a = H 0 + length 10.
    #[test]
    fn decodes_the_name_of_c_2_1() {
        assert_eq!(
            decode_string(&literal(b"custom-key")),
            Ok((b"custom-key".to_vec(), 11))
        );
    }

    // --- Length ---

    // An empty value is legal (e.g. "accept-encoding: ").
    #[test]
    fn decodes_an_empty_string() {
        assert_eq!(decode_string(&[0x00]), Ok((vec![], 1)));
    }

    // 200 = 127 + 73: the length overflows its 7-bit prefix into a continuation byte.
    #[test]
    fn decodes_a_length_with_a_continuation_byte() {
        let mut input = vec![0b0111_1111, 73];
        input.extend_from_slice(&[b'a'; 200]);

        assert_eq!(decode_string(&input), Ok((vec![b'a'; 200], 202)));
    }

    // What follows the string belongs to the next field: it must not be consumed.
    #[test]
    fn stops_after_the_string() {
        let mut input = literal(b"no-cache");
        input.extend_from_slice(&[0x82, 0x86]);

        assert_eq!(decode_string(&input), Ok((b"no-cache".to_vec(), 9)));
    }

    // --- Content ---

    // Header values are bytes, not necessarily UTF-8 (RFC 9110 §5.5 obs-text).
    #[test]
    fn keeps_non_utf8_bytes() {
        assert_eq!(
            decode_string(&[0x02, 0xff, 0xfe]),
            Ok((vec![0xff, 0xfe], 3))
        );
    }

    // --- H bit ---

    // C.4.1: 0x8c = H 1 + length 12. The H bit is never part of the length.
    #[test]
    fn huffman_strings_are_not_supported_yet() {
        let input = [
            0x8c, 0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab, 0x90, 0xf4, 0xff,
        ];

        assert_eq!(decode_string(&input), Err(HpackError::HuffmanUnsupported));
    }

    // --- Errors ---

    #[test]
    fn empty_input_is_truncated() {
        assert_eq!(decode_string(&[]), Err(HpackError::Truncated));
    }

    // The length announces 15 bytes, only 3 are there.
    #[test]
    fn data_shorter_than_the_length_is_truncated() {
        let mut input = vec![0x0f];
        input.extend_from_slice(b"www");

        assert_eq!(decode_string(&input), Err(HpackError::Truncated));
    }

    // A peer announcing ~268 MB with a few bytes must not make us allocate them.
    #[test]
    fn huge_announced_length_is_truncated() {
        let input = [0b0111_1111, 0xFF, 0xFF, 0xFF, 0x7F, b'a'];

        assert_eq!(decode_string(&input), Err(HpackError::Truncated));
    }

    // Errors from the length integer are propagated as is.
    #[test]
    fn length_overflow_is_propagated() {
        let mut input = vec![0b0111_1111];
        input.extend_from_slice(&[0xFF; 10]);
        input.push(0x01);

        assert_eq!(decode_string(&input), Err(HpackError::Overflow));
    }
}
