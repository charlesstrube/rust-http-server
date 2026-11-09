use crate::http2::hpack::HpackError;

const MAX_CONTINUATION_BYTES: usize = 5;
const CHUNK_BITS: usize = 7;
const CHUNK_MASK: u8 = 0x7F; // 0111_1111 : the 7 value bits
const CONTINUATION_FLAG: u8 = 0x80;

/*
 * Format	                 result
 * {:08b}	                01000001
 * {:#010b}	                0b01000001 (the 0b prefix counts in the 10 characters)
 * {:02x}	                41
 * {:#04x}	                0x41
 * {:02x?} on a slice	    [41, 0f, 77, 77]
*/

// RFC 7541 §5.1: decodes an integer whose first byte uses its `prefix_bits`
// low bits (1..=8). Returns the value and the number of bytes consumed.
//
// The bits above the prefix are flags owned by the caller: they are ignored.
// If the prefix is not all ones, it holds the whole value:
//
//     0 1 0 │ 0 1 0 1 0                prefix_bits = 5  →  (10, 1)
//     flags │ value = 10
//
// If the prefix is all ones (2^N - 1), the value goes on in the next bytes:
// 7 value bits each, least significant group first, bit 7 = "another byte follows".
//
//     byte 0   0 0 0 │ 1 1 1 1 1       prefix full             =   31
//     byte 1   1 │ 0 0 1 1 0 1 0       go on,  26 << (0 * 7)  =   26
//     byte 2   0 │ 0 0 0 1 0 1 0       last,   10 << (1 * 7)  = 1280
//                                                              ──────
//                                                   (1337, 3) ←  1337
//
// At most MAX_CONTINUATION_BYTES follow: this bounds the shift and rejects
// a peer sending endless continuation bytes.
pub fn decode_integer(input: &[u8], prefix_bits: u8) -> Result<(usize, usize), HpackError> {
    let first = input.first().ok_or(HpackError::InvalidTruncated)?;

    let mask = u8::MAX >> (8 - prefix_bits);
    let value = first & mask;

    let mut result = value as usize;

    if value < mask {
        return Ok((result, 1));
    }

    for (index, byte) in input[1..].iter().enumerate() {
        if index >= MAX_CONTINUATION_BYTES {
            return Err(HpackError::InvalidOverflow);
        }
        let count = index + 2; // we are on the second chunk
        let chunk = byte & CHUNK_MASK;
        let moving = index * CHUNK_BITS;

        result = result
            .checked_add((chunk as usize) << moving)
            .ok_or(HpackError::InvalidOverflow)?;

        // if 1, we continue, if it's 0 then it's the last byte
        if byte & CONTINUATION_FLAG == 0 {
            return Ok((result, count));
        }
    }
    Err(HpackError::InvalidTruncated)
}

// RFC 7541 §5.1: appends `value` to `out` with a `prefix_bits` prefix (1..=8).
// `flags` holds the representation bits above the prefix (e.g. 0x80 for indexed).
pub fn encode_integer(value: usize, prefix_bits: u8, flags: u8, out: &mut Vec<u8>) {
    let mask = u8::MAX >> (8 - prefix_bits);
    if value < usize::from(mask) {
        let test = flags | value as u8;
        out.push(test);
    } else {
        let mut rest = value - usize::from(mask);
        out.push(flags | mask);
        while rest >= 128 {
            let current = rest & 0x7F;
            let mut byte = current as u8;
            byte = byte | 0x80;
            rest >>= 7;
            out.push(byte);
        }

        // after the loop, push the rest
        out.push(rest as u8);
    }

    // todo!("step 5.1")
}

#[cfg(test)]
mod test {
    use super::*;

    // --- RFC 7541 Appendix C.1 ---

    // C.1.1: fits in the prefix (10 < 2^5 - 1).
    #[test]
    fn decodes_10_with_a_5_bit_prefix() {
        assert_eq!(decode_integer(&[0b0000_1010], 5), Ok((10, 1)));
    }

    // C.1.2: 1337 = 31 + 26 + 10 * 128.
    #[test]
    fn decodes_1337_with_a_5_bit_prefix() {
        assert_eq!(
            decode_integer(&[0b0001_1111, 0b1001_1010, 0b0000_1010], 5),
            Ok((1337, 3))
        );
    }

    // C.1.3: an 8-bit prefix uses the whole first byte.
    #[test]
    fn decodes_42_with_an_8_bit_prefix() {
        assert_eq!(decode_integer(&[0b0010_1010], 8), Ok((42, 1)));
    }

    // --- Prefix boundaries ---

    // The bits above the prefix belong to the representation (flags), not to the value.
    #[test]
    fn ignores_the_bits_above_the_prefix() {
        assert_eq!(decode_integer(&[0b1110_1010], 5), Ok((10, 1)));
    }

    #[test]
    fn largest_value_that_fits_in_the_prefix() {
        assert_eq!(decode_integer(&[0b0001_1110], 5), Ok((30, 1)));
    }

    // A prefix full of ones (2^N - 1) always means "continuation follows",
    // even when the value is exactly 2^N - 1.
    #[test]
    fn prefix_full_of_ones_needs_a_continuation_byte() {
        assert_eq!(decode_integer(&[0b0001_1111, 0b0000_0000], 5), Ok((31, 2)));
    }

    // String lengths use a 7-bit prefix (RFC 7541 §5.2).
    #[test]
    fn decodes_128_with_a_7_bit_prefix() {
        assert_eq!(decode_integer(&[0b0111_1111, 0b0000_0001], 7), Ok((128, 2)));
    }

    // Literal header fields use a 6-bit prefix (RFC 7541 §6.2.1).
    #[test]
    fn decodes_63_with_a_6_bit_prefix() {
        assert_eq!(decode_integer(&[0b0011_1111, 0b0000_0000], 6), Ok((63, 2)));
    }

    // A useless continuation byte (0x80 = 0 with the continuation flag) is still valid.
    #[test]
    fn accepts_redundant_continuation_bytes() {
        assert_eq!(
            decode_integer(&[0b0001_1111, 0b1000_0000, 0b0000_0000], 5),
            Ok((31, 3))
        );
    }

    // --- Consumed bytes ---

    // What follows the integer belongs to the next field: it must not be consumed.
    #[test]
    fn stops_after_the_integer() {
        assert_eq!(decode_integer(&[0b0000_1010, 0xFF, 0xFF], 5), Ok((10, 1)));
    }

    #[test]
    fn stops_after_the_last_continuation_byte() {
        assert_eq!(
            decode_integer(&[0b0001_1111, 0b1001_1010, 0b0000_1010, 0xFF], 5),
            Ok((1337, 3))
        );
    }

    // --- Errors ---

    #[test]
    fn empty_input_is_truncated() {
        assert_eq!(decode_integer(&[], 5), Err(HpackError::InvalidTruncated));
    }

    #[test]
    fn missing_continuation_byte_is_truncated() {
        assert_eq!(
            decode_integer(&[0b0001_1111], 5),
            Err(HpackError::InvalidTruncated)
        );
    }

    // The last byte still has its continuation bit (0x80) set.
    #[test]
    fn unfinished_continuation_is_truncated() {
        assert_eq!(
            decode_integer(&[0b0001_1111, 0b1001_1010], 5),
            Err(HpackError::InvalidTruncated)
        );
    }

    // A peer can send endless continuation bytes: never panic, never wrap around.
    #[test]
    fn too_many_continuation_bytes_overflow() {
        let mut input = vec![0b0001_1111];
        input.extend_from_slice(&[0xFF; 10]);
        input.push(0b0000_0001);

        assert_eq!(decode_integer(&input, 5), Err(HpackError::InvalidOverflow));
    }

    // The limit itself is accepted: 5 continuation bytes, the last chunk shifted by 28.
    #[test]
    fn accepts_the_maximum_number_of_continuation_bytes() {
        let input = [0b0001_1111, 0x80, 0x80, 0x80, 0x80, 0b0000_0001];

        assert_eq!(decode_integer(&input, 5), Ok((31 + (1 << 28), 6)));
    }

    // One byte past the limit overflows, even though the value would fit in a usize.
    #[test]
    fn rejects_one_continuation_byte_past_the_limit() {
        let input = [0b0001_1111, 0x80, 0x80, 0x80, 0x80, 0x80, 0b0000_0001];

        assert_eq!(decode_integer(&input, 5), Err(HpackError::InvalidOverflow));
    }

    // --- Encoding (step 5.1) ---

    fn encoded(value: usize, prefix_bits: u8, flags: u8) -> Vec<u8> {
        let mut out = vec![];
        encode_integer(value, prefix_bits, flags, &mut out);
        out
    }

    // C.1.1
    #[test]
    fn encodes_10_with_a_5_bit_prefix() {
        assert_eq!(encoded(10, 5, 0), vec![0b0000_1010]);
    }

    // C.1.2: 1337 - 31 = 1306 = 10 * 128 + 26, least significant group first.
    #[test]
    fn encodes_1337_with_a_5_bit_prefix() {
        assert_eq!(
            encoded(1337, 5, 0),
            vec![0b0001_1111, 0b1001_1010, 0b0000_1010]
        );
    }

    // C.1.3
    #[test]
    fn encodes_42_with_an_8_bit_prefix() {
        assert_eq!(encoded(42, 8, 0), vec![0b0010_1010]);
    }

    #[test]
    fn encodes_zero() {
        assert_eq!(encoded(0, 7, 0), vec![0x00]);
    }

    #[test]
    fn encodes_the_largest_value_that_fits_in_the_prefix() {
        assert_eq!(encoded(30, 5, 0), vec![0b0001_1110]);
    }

    // The trap: 2^N - 1 does not fit, a prefix full of ones always announces
    // a continuation, so a 0x00 byte must follow.
    #[test]
    fn value_equal_to_the_full_prefix_needs_a_zero_continuation_byte() {
        assert_eq!(encoded(31, 5, 0), vec![0b0001_1111, 0b0000_0000]);
        assert_eq!(encoded(127, 7, 0), vec![0b0111_1111, 0b0000_0000]);
        assert_eq!(encoded(255, 8, 0), vec![0b1111_1111, 0b0000_0000]);
    }

    #[test]
    fn encodes_128_with_a_7_bit_prefix() {
        assert_eq!(encoded(128, 7, 0), vec![0b0111_1111, 0b0000_0001]);
    }

    // 127 + 128 = 255: the remainder (128) needs two 7-bit groups.
    #[test]
    fn encodes_a_remainder_on_two_continuation_bytes() {
        assert_eq!(
            encoded(255, 7, 0),
            vec![0b0111_1111, 0b1000_0000, 0b0000_0001]
        );
    }

    // The flags only touch the bits above the prefix, on the first byte only.
    #[test]
    fn keeps_the_flags_above_the_prefix() {
        assert_eq!(encoded(10, 5, 0b1110_0000), vec![0b1110_1010]);
        assert_eq!(
            encoded(1337, 5, 0b1110_0000),
            vec![0b1111_1111, 0b1001_1010, 0b0000_1010]
        );
    }

    // C.2.4: indexed field 2 (:method GET) = flag 0x80 + 7-bit prefix.
    #[test]
    fn encodes_an_indexed_field_byte() {
        assert_eq!(encoded(2, 7, 0x80), vec![0x82]);
    }

    // Appends: what is already in `out` stays in front.
    #[test]
    fn appends_to_the_output() {
        let mut out = vec![0xAA];
        encode_integer(10, 5, 0, &mut out);

        assert_eq!(out, vec![0xAA, 0b0000_1010]);
    }

    // Whatever the encoder writes, the decoder of step 4 must read back.
    #[test]
    fn round_trips_through_decode_integer() {
        for prefix_bits in 1..=8 {
            for value in (0..2_000).chain([16_383, 16_384, 65_535, 1 << 20]) {
                let bytes = encoded(value, prefix_bits, 0);

                assert_eq!(
                    decode_integer(&bytes, prefix_bits),
                    Ok((value, bytes.len())),
                    "value {value} with a {prefix_bits}-bit prefix"
                );
            }
        }
    }
}
