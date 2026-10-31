use crate::http2::hpack::HpackError;

/*
 * Format	                 result
 * {:08b}	                01000001
 * {:#010b}	                0b01000001 (le préfixe 0b compte dans les 10 caractères)
 * {:02x}	                41
 * {:#04x}	                0x41
 * {:02x?} sur un slice	    [41, 0f, 77, 77]
*/

// RFC 7541 §5.1: decodes an integer whose first byte uses its `prefix_bits`
// low bits (1..=8). Returns the value and the number of bytes consumed.
pub fn decode_integer(input: &[u8], prefix_bits: u8) -> Result<(usize, usize), HpackError> {
    let mut count = 1;

    let mut moving = 0;
    let first = input.first().ok_or(HpackError::Truncated)?;

    let mask = u8::MAX >> (8 - prefix_bits);
    let value = first & mask;

    let mut result = value as usize;

    if value < mask {
        return Ok((value as usize, count));
    }

    for (index, byte) in input[1..].iter().enumerate() {
        count += 1;
        let bit7 = byte >> 7;
        let chunk = byte & 0x7F;

        result += (chunk as usize) << moving;

        if bit7 == 0 {
            return Ok((result, count));
        }

        moving += 7;
        println!("{} - byte: {:08b}", index, byte);
    }
    Err(HpackError::Truncated)
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
        assert_eq!(decode_integer(&[], 5), Err(HpackError::Truncated));
    }

    #[test]
    fn missing_continuation_byte_is_truncated() {
        assert_eq!(
            decode_integer(&[0b0001_1111], 5),
            Err(HpackError::Truncated)
        );
    }

    // The last byte still has its continuation bit (0x80) set.
    #[test]
    fn unfinished_continuation_is_truncated() {
        assert_eq!(
            decode_integer(&[0b0001_1111, 0b1001_1010], 5),
            Err(HpackError::Truncated)
        );
    }

    // A peer can send endless continuation bytes: never panic, never wrap around.
    #[test]
    fn too_many_continuation_bytes_overflow() {
        let mut input = vec![0b0001_1111];
        input.extend_from_slice(&[0xFF; 10]);
        input.push(0b0000_0001);

        assert_eq!(decode_integer(&input, 5), Err(HpackError::Overflow));
    }
}
