use std::sync::LazyLock;

use crate::http2::hpack::HpackError;

// RFC 7541 Appendix B: the Huffman code of every symbol, as (code, bit length).
// The index is the symbol: 0..=255 are bytes, 256 is EOS (end of string).
// The code is right-aligned: 'a' = (0x3, 5) means the 5 bits 00011.
//
// Generated from the RFC text, checked prefix-free and against the C.4 / C.6
// examples. This is data, not logic: the decoding is up to you.
pub const CODES: [(u32, usize); 257] = [
    (0x00001ff8, 13), //   0 0x00
    (0x007fffd8, 23), //   1 0x01
    (0x0fffffe2, 28), //   2 0x02
    (0x0fffffe3, 28), //   3 0x03
    (0x0fffffe4, 28), //   4 0x04
    (0x0fffffe5, 28), //   5 0x05
    (0x0fffffe6, 28), //   6 0x06
    (0x0fffffe7, 28), //   7 0x07
    (0x0fffffe8, 28), //   8 0x08
    (0x00ffffea, 24), //   9 0x09
    (0x3ffffffc, 30), //  10 0x0a
    (0x0fffffe9, 28), //  11 0x0b
    (0x0fffffea, 28), //  12 0x0c
    (0x3ffffffd, 30), //  13 0x0d
    (0x0fffffeb, 28), //  14 0x0e
    (0x0fffffec, 28), //  15 0x0f
    (0x0fffffed, 28), //  16 0x10
    (0x0fffffee, 28), //  17 0x11
    (0x0fffffef, 28), //  18 0x12
    (0x0ffffff0, 28), //  19 0x13
    (0x0ffffff1, 28), //  20 0x14
    (0x0ffffff2, 28), //  21 0x15
    (0x3ffffffe, 30), //  22 0x16
    (0x0ffffff3, 28), //  23 0x17
    (0x0ffffff4, 28), //  24 0x18
    (0x0ffffff5, 28), //  25 0x19
    (0x0ffffff6, 28), //  26 0x1a
    (0x0ffffff7, 28), //  27 0x1b
    (0x0ffffff8, 28), //  28 0x1c
    (0x0ffffff9, 28), //  29 0x1d
    (0x0ffffffa, 28), //  30 0x1e
    (0x0ffffffb, 28), //  31 0x1f
    (0x00000014, 6),  //  32 ' '
    (0x000003f8, 10), //  33 '!'
    (0x000003f9, 10), //  34 '"'
    (0x00000ffa, 12), //  35 '#'
    (0x00001ff9, 13), //  36 '$'
    (0x00000015, 6),  //  37 '%'
    (0x000000f8, 8),  //  38 '&'
    (0x000007fa, 11), //  39 '\''
    (0x000003fa, 10), //  40 '('
    (0x000003fb, 10), //  41 ')'
    (0x000000f9, 8),  //  42 '*'
    (0x000007fb, 11), //  43 '+'
    (0x000000fa, 8),  //  44 ','
    (0x00000016, 6),  //  45 '-'
    (0x00000017, 6),  //  46 '.'
    (0x00000018, 6),  //  47 '/'
    (0x00000000, 5),  //  48 '0'
    (0x00000001, 5),  //  49 '1'
    (0x00000002, 5),  //  50 '2'
    (0x00000019, 6),  //  51 '3'
    (0x0000001a, 6),  //  52 '4'
    (0x0000001b, 6),  //  53 '5'
    (0x0000001c, 6),  //  54 '6'
    (0x0000001d, 6),  //  55 '7'
    (0x0000001e, 6),  //  56 '8'
    (0x0000001f, 6),  //  57 '9'
    (0x0000005c, 7),  //  58 ':'
    (0x000000fb, 8),  //  59 ';'
    (0x00007ffc, 15), //  60 '<'
    (0x00000020, 6),  //  61 '='
    (0x00000ffb, 12), //  62 '>'
    (0x000003fc, 10), //  63 '?'
    (0x00001ffa, 13), //  64 '@'
    (0x00000021, 6),  //  65 'A'
    (0x0000005d, 7),  //  66 'B'
    (0x0000005e, 7),  //  67 'C'
    (0x0000005f, 7),  //  68 'D'
    (0x00000060, 7),  //  69 'E'
    (0x00000061, 7),  //  70 'F'
    (0x00000062, 7),  //  71 'G'
    (0x00000063, 7),  //  72 'H'
    (0x00000064, 7),  //  73 'I'
    (0x00000065, 7),  //  74 'J'
    (0x00000066, 7),  //  75 'K'
    (0x00000067, 7),  //  76 'L'
    (0x00000068, 7),  //  77 'M'
    (0x00000069, 7),  //  78 'N'
    (0x0000006a, 7),  //  79 'O'
    (0x0000006b, 7),  //  80 'P'
    (0x0000006c, 7),  //  81 'Q'
    (0x0000006d, 7),  //  82 'R'
    (0x0000006e, 7),  //  83 'S'
    (0x0000006f, 7),  //  84 'T'
    (0x00000070, 7),  //  85 'U'
    (0x00000071, 7),  //  86 'V'
    (0x00000072, 7),  //  87 'W'
    (0x000000fc, 8),  //  88 'X'
    (0x00000073, 7),  //  89 'Y'
    (0x000000fd, 8),  //  90 'Z'
    (0x00001ffb, 13), //  91 '['
    (0x0007fff0, 19), //  92 '\\'
    (0x00001ffc, 13), //  93 ']'
    (0x00003ffc, 14), //  94 '^'
    (0x00000022, 6),  //  95 '_'
    (0x00007ffd, 15), //  96 '`'
    (0x00000003, 5),  //  97 'a'
    (0x00000023, 6),  //  98 'b'
    (0x00000004, 5),  //  99 'c'
    (0x00000024, 6),  // 100 'd'
    (0x00000005, 5),  // 101 'e'
    (0x00000025, 6),  // 102 'f'
    (0x00000026, 6),  // 103 'g'
    (0x00000027, 6),  // 104 'h'
    (0x00000006, 5),  // 105 'i'
    (0x00000074, 7),  // 106 'j'
    (0x00000075, 7),  // 107 'k'
    (0x00000028, 6),  // 108 'l'
    (0x00000029, 6),  // 109 'm'
    (0x0000002a, 6),  // 110 'n'
    (0x00000007, 5),  // 111 'o'
    (0x0000002b, 6),  // 112 'p'
    (0x00000076, 7),  // 113 'q'
    (0x0000002c, 6),  // 114 'r'
    (0x00000008, 5),  // 115 's'
    (0x00000009, 5),  // 116 't'
    (0x0000002d, 6),  // 117 'u'
    (0x00000077, 7),  // 118 'v'
    (0x00000078, 7),  // 119 'w'
    (0x00000079, 7),  // 120 'x'
    (0x0000007a, 7),  // 121 'y'
    (0x0000007b, 7),  // 122 'z'
    (0x00007ffe, 15), // 123 '{'
    (0x000007fc, 11), // 124 '|'
    (0x00003ffd, 14), // 125 '}'
    (0x00001ffd, 13), // 126 '~'
    (0x0ffffffc, 28), // 127 0x7f
    (0x000fffe6, 20), // 128 0x80
    (0x003fffd2, 22), // 129 0x81
    (0x000fffe7, 20), // 130 0x82
    (0x000fffe8, 20), // 131 0x83
    (0x003fffd3, 22), // 132 0x84
    (0x003fffd4, 22), // 133 0x85
    (0x003fffd5, 22), // 134 0x86
    (0x007fffd9, 23), // 135 0x87
    (0x003fffd6, 22), // 136 0x88
    (0x007fffda, 23), // 137 0x89
    (0x007fffdb, 23), // 138 0x8a
    (0x007fffdc, 23), // 139 0x8b
    (0x007fffdd, 23), // 140 0x8c
    (0x007fffde, 23), // 141 0x8d
    (0x00ffffeb, 24), // 142 0x8e
    (0x007fffdf, 23), // 143 0x8f
    (0x00ffffec, 24), // 144 0x90
    (0x00ffffed, 24), // 145 0x91
    (0x003fffd7, 22), // 146 0x92
    (0x007fffe0, 23), // 147 0x93
    (0x00ffffee, 24), // 148 0x94
    (0x007fffe1, 23), // 149 0x95
    (0x007fffe2, 23), // 150 0x96
    (0x007fffe3, 23), // 151 0x97
    (0x007fffe4, 23), // 152 0x98
    (0x001fffdc, 21), // 153 0x99
    (0x003fffd8, 22), // 154 0x9a
    (0x007fffe5, 23), // 155 0x9b
    (0x003fffd9, 22), // 156 0x9c
    (0x007fffe6, 23), // 157 0x9d
    (0x007fffe7, 23), // 158 0x9e
    (0x00ffffef, 24), // 159 0x9f
    (0x003fffda, 22), // 160 0xa0
    (0x001fffdd, 21), // 161 0xa1
    (0x000fffe9, 20), // 162 0xa2
    (0x003fffdb, 22), // 163 0xa3
    (0x003fffdc, 22), // 164 0xa4
    (0x007fffe8, 23), // 165 0xa5
    (0x007fffe9, 23), // 166 0xa6
    (0x001fffde, 21), // 167 0xa7
    (0x007fffea, 23), // 168 0xa8
    (0x003fffdd, 22), // 169 0xa9
    (0x003fffde, 22), // 170 0xaa
    (0x00fffff0, 24), // 171 0xab
    (0x001fffdf, 21), // 172 0xac
    (0x003fffdf, 22), // 173 0xad
    (0x007fffeb, 23), // 174 0xae
    (0x007fffec, 23), // 175 0xaf
    (0x001fffe0, 21), // 176 0xb0
    (0x001fffe1, 21), // 177 0xb1
    (0x003fffe0, 22), // 178 0xb2
    (0x001fffe2, 21), // 179 0xb3
    (0x007fffed, 23), // 180 0xb4
    (0x003fffe1, 22), // 181 0xb5
    (0x007fffee, 23), // 182 0xb6
    (0x007fffef, 23), // 183 0xb7
    (0x000fffea, 20), // 184 0xb8
    (0x003fffe2, 22), // 185 0xb9
    (0x003fffe3, 22), // 186 0xba
    (0x003fffe4, 22), // 187 0xbb
    (0x007ffff0, 23), // 188 0xbc
    (0x003fffe5, 22), // 189 0xbd
    (0x003fffe6, 22), // 190 0xbe
    (0x007ffff1, 23), // 191 0xbf
    (0x03ffffe0, 26), // 192 0xc0
    (0x03ffffe1, 26), // 193 0xc1
    (0x000fffeb, 20), // 194 0xc2
    (0x0007fff1, 19), // 195 0xc3
    (0x003fffe7, 22), // 196 0xc4
    (0x007ffff2, 23), // 197 0xc5
    (0x003fffe8, 22), // 198 0xc6
    (0x01ffffec, 25), // 199 0xc7
    (0x03ffffe2, 26), // 200 0xc8
    (0x03ffffe3, 26), // 201 0xc9
    (0x03ffffe4, 26), // 202 0xca
    (0x07ffffde, 27), // 203 0xcb
    (0x07ffffdf, 27), // 204 0xcc
    (0x03ffffe5, 26), // 205 0xcd
    (0x00fffff1, 24), // 206 0xce
    (0x01ffffed, 25), // 207 0xcf
    (0x0007fff2, 19), // 208 0xd0
    (0x001fffe3, 21), // 209 0xd1
    (0x03ffffe6, 26), // 210 0xd2
    (0x07ffffe0, 27), // 211 0xd3
    (0x07ffffe1, 27), // 212 0xd4
    (0x03ffffe7, 26), // 213 0xd5
    (0x07ffffe2, 27), // 214 0xd6
    (0x00fffff2, 24), // 215 0xd7
    (0x001fffe4, 21), // 216 0xd8
    (0x001fffe5, 21), // 217 0xd9
    (0x03ffffe8, 26), // 218 0xda
    (0x03ffffe9, 26), // 219 0xdb
    (0x0ffffffd, 28), // 220 0xdc
    (0x07ffffe3, 27), // 221 0xdd
    (0x07ffffe4, 27), // 222 0xde
    (0x07ffffe5, 27), // 223 0xdf
    (0x000fffec, 20), // 224 0xe0
    (0x00fffff3, 24), // 225 0xe1
    (0x000fffed, 20), // 226 0xe2
    (0x001fffe6, 21), // 227 0xe3
    (0x003fffe9, 22), // 228 0xe4
    (0x001fffe7, 21), // 229 0xe5
    (0x001fffe8, 21), // 230 0xe6
    (0x007ffff3, 23), // 231 0xe7
    (0x003fffea, 22), // 232 0xe8
    (0x003fffeb, 22), // 233 0xe9
    (0x01ffffee, 25), // 234 0xea
    (0x01ffffef, 25), // 235 0xeb
    (0x00fffff4, 24), // 236 0xec
    (0x00fffff5, 24), // 237 0xed
    (0x03ffffea, 26), // 238 0xee
    (0x007ffff4, 23), // 239 0xef
    (0x03ffffeb, 26), // 240 0xf0
    (0x07ffffe6, 27), // 241 0xf1
    (0x03ffffec, 26), // 242 0xf2
    (0x03ffffed, 26), // 243 0xf3
    (0x07ffffe7, 27), // 244 0xf4
    (0x07ffffe8, 27), // 245 0xf5
    (0x07ffffe9, 27), // 246 0xf6
    (0x07ffffea, 27), // 247 0xf7
    (0x07ffffeb, 27), // 248 0xf8
    (0x0ffffffe, 28), // 249 0xf9
    (0x07ffffec, 27), // 250 0xfa
    (0x07ffffed, 27), // 251 0xfb
    (0x07ffffee, 27), // 252 0xfc
    (0x07ffffef, 27), // 253 0xfd
    (0x07fffff0, 27), // 254 0xfe
    (0x03ffffee, 26), // 255 0xff
    (0x3fffffff, 30), // 256 EOS
];

#[derive(Default, Debug, PartialEq)]
struct Node {
    children: [Option<usize>; 2],
    symbol: Option<u16>,
}

fn build_btree() -> Vec<Node> {
    let mut tree = vec![Node::default()];
    for (code_index, (code, length)) in CODES.iter().enumerate() {
        let mut current_index = 0;
        for index in (0..*length).rev() {
            let bit = (code >> index) & 1;
            let children_index = bit as usize;
            match tree[current_index].children[children_index] {
                Some(sub_node_index) => current_index = sub_node_index,
                None => {
                    let mut node = Node::default();
                    tree[current_index].children[children_index] = Some(tree.len());
                    current_index = tree.len();
                    if index == 0 {
                        node.symbol = Some(code_index as u16);
                        current_index = 0;
                    }

                    tree.push(node);
                }
            }
        }
    }

    tree
}

static TREE: LazyLock<Vec<Node>> = LazyLock::new(build_btree);

// RFC 7541 §5.2: decodes Huffman-coded string data (the bytes after the length).
//
// Bits are read most significant first. The data ends with 0 to 7 padding bits,
// all set to 1 (the start of EOS). Errors:
// - padding longer than 7 bits, or not made only of 1s → InvalidHuffmanPadding
// - the EOS symbol decoded inside the data              → HuffmanEos

pub fn decode_huffman_btree(input: &[u8]) -> Result<Vec<u8>, HpackError> {
    let mut symbols: Vec<u8> = vec![];
    let mut nbits: usize = 0;
    let mut has_0s = false;
    let mut current_index = 0;

    for byte in input.iter() {
        for bit_index in (0..8).rev() {
            let bit = (byte >> bit_index) & 1;
            nbits += 1;

            if bit == 0 {
                has_0s = true
            }

            let node_index = TREE[current_index].children[bit as usize].expect("complete code");
            if TREE[node_index].symbol.is_some() {
                let symbol = TREE[node_index].symbol.expect("complete code");
                symbols.push(u8::try_from(symbol).map_err(|_| HpackError::HuffmanEos)?);
                current_index = 0;
                nbits = 0;
                has_0s = false;
            } else {
                current_index = node_index;
            }
        }
    }

    if nbits >= 8 && !has_0s || nbits != 0 && has_0s {
        return Err(HpackError::InvalidHuffmanPadding);
    }

    Ok(symbols)
}

pub fn decode_huffman(input: &[u8]) -> Result<Vec<u8>, HpackError> {
    let mut acc: u32 = 0;
    let mut symbols: Vec<u8> = vec![];
    let mut nbits: usize = 0;
    let mut has_0s = false;
    for byte in input.iter() {
        for index in (0..8).rev() {
            let bit = (byte >> index) & 1;
            acc = (acc << 1) | u32::from(bit);
            nbits += 1;

            if bit == 0 {
                has_0s = true
            }

            for (symbol, (code, len)) in CODES.iter().enumerate() {
                if (code, len) == (&acc, &nbits) {
                    symbols.push(u8::try_from(symbol).map_err(|_| HpackError::HuffmanEos)?);
                    acc = 0;
                    nbits = 0;
                    has_0s = false;
                    break;
                }
            }
        }
    }

    if nbits >= 8 && !has_0s || nbits != 0 && has_0s {
        return Err(HpackError::InvalidHuffmanPadding);
    }

    Ok(symbols)
}

#[cfg(test)]
mod test {
    use super::*;

    fn decoded(hex: &[u8]) -> Result<String, HpackError> {
        decode_huffman_btree(hex).map(|bytes| String::from_utf8(bytes).unwrap())
    }

    // --- The table ---

    // Frequent characters get the shortest codes (5 bits).
    #[test]
    fn frequent_symbols_have_5_bit_codes() {
        assert_eq!(CODES[b'0' as usize], (0x0, 5)); // 00000
        assert_eq!(CODES[b'a' as usize], (0x3, 5)); // 00011
        assert_eq!(CODES[b'e' as usize], (0x5, 5)); // 00101
    }

    // EOS is 30 bits, all set to 1: padding is always a prefix of it.
    #[test]
    fn eos_is_thirty_ones() {
        assert_eq!(CODES[256], (0x3fff_ffff, 30));
    }

    // --- RFC 7541 Appendix C.4 (requests) ---

    // C.4.1: 15 characters in 12 bytes.
    #[test]
    fn decodes_www_example_com() {
        let data = [
            0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab, 0x90, 0xf4, 0xff,
        ];

        assert_eq!(decoded(&data), Ok("www.example.com".to_string()));
    }

    // C.4.2
    #[test]
    fn decodes_no_cache() {
        let data = [0xa8, 0xeb, 0x10, 0x64, 0x9c, 0xbf];

        assert_eq!(decoded(&data), Ok("no-cache".to_string()));
    }

    // C.4.3: name and value share their first 5 bytes ("custom-").
    #[test]
    fn decodes_custom_key_and_custom_value() {
        let key = [0x25, 0xa8, 0x49, 0xe9, 0x5b, 0xa9, 0x7d, 0x7f];
        let value = [0x25, 0xa8, 0x49, 0xe9, 0x5b, 0xb8, 0xe8, 0xb4, 0xbf];

        assert_eq!(decoded(&key), Ok("custom-key".to_string()));
        assert_eq!(decoded(&value), Ok("custom-value".to_string()));
    }

    // --- RFC 7541 Appendix C.6 (responses) ---

    // Digits, ',', ':' and uppercase letters: codes from 5 to 8 bits across byte boundaries.
    #[test]
    fn decodes_a_date() {
        let data = [
            0xd0, 0x7a, 0xbe, 0x94, 0x10, 0x54, 0xd4, 0x44, 0xa8, 0x20, 0x05, 0x95, 0x04, 0x0b,
            0x81, 0x66, 0xe0, 0x82, 0xa6, 0x2d, 0x1b, 0xff,
        ];

        assert_eq!(
            decoded(&data),
            Ok("Mon, 21 Oct 2013 20:13:21 GMT".to_string())
        );
    }

    #[test]
    fn decodes_a_url() {
        let data = [
            0x9d, 0x29, 0xad, 0x17, 0x18, 0x63, 0xc7, 0x8f, 0x0b, 0x97, 0xc8, 0xe9, 0xae, 0x82,
            0xae, 0x43, 0xd3,
        ];

        assert_eq!(decoded(&data), Ok("https://www.example.com".to_string()));
    }

    // C.6.1 and C.6.2: short values, 2 to 3 bits of padding.
    #[test]
    fn decodes_status_codes() {
        assert_eq!(decoded(&[0x64, 0x02]), Ok("302".to_string()));
        assert_eq!(decoded(&[0x64, 0x0e, 0xff]), Ok("307".to_string()));
    }

    // --- Boundaries ---

    #[test]
    fn empty_data_is_an_empty_string() {
        assert_eq!(decoded(&[]), Ok(String::new()));
    }

    // 'a' = 00011, then 3 bits of padding: 00011_111.
    #[test]
    fn single_symbol_with_padding() {
        assert_eq!(decoded(&[0b0001_1111]), Ok("a".to_string()));
    }

    // 'w' = 1111000 (7 bits), then 1 bit of padding: the shortest valid padding.
    #[test]
    fn single_bit_of_padding() {
        assert_eq!(decoded(&[0b1111_0001]), Ok("w".to_string()));
    }

    // '0' = 00000: a symbol made of 0s, then 111. Only the padding must be 1s.
    #[test]
    fn symbol_of_zeros_with_padding() {
        assert_eq!(decoded(&[0b0000_0111]), Ok("0".to_string()));
    }

    // '&' = 11111000 (8 bits): the data ends exactly on a byte, no padding.
    #[test]
    fn symbol_ending_on_a_byte_boundary() {
        assert_eq!(decoded(&[0b1111_1000]), Ok("&".to_string()));
    }

    // A code can span several bytes: '\\' = 19 bits, 0x7fff0, then 5 bits of padding.
    #[test]
    fn symbol_spanning_three_bytes() {
        assert_eq!(decoded(&[0xff, 0xfe, 0x1f]), Ok("\\".to_string()));
    }

    // Symbol 0xff (26 bits) is not UTF-8 on its own: Huffman decodes bytes, not text.
    #[test]
    fn decodes_non_utf8_bytes() {
        // 0x3ffffee on 26 bits, then 6 bits of padding.
        assert_eq!(
            decode_huffman_btree(&[0xff, 0xff, 0xfb, 0xbf]),
            Ok(vec![0xff])
        );
    }

    // --- Errors (RFC 7541 §5.2) ---

    // 'a' then 110: padding must be made only of 1s.
    #[test]
    fn padding_with_a_zero_is_rejected() {
        assert_eq!(
            decode_huffman_btree(&[0b0001_1110]),
            Err(HpackError::InvalidHuffmanPadding)
        );
    }

    // 'a' then 000: a padding made only of 0s is still not made of 1s.
    #[test]
    fn padding_of_zeros_is_rejected() {
        assert_eq!(
            decode_huffman_btree(&[0b0001_1000]),
            Err(HpackError::InvalidHuffmanPadding)
        );
    }

    // 'w' = 1111000 (7 bits), then a single 0: even 1 bit of padding must be a 1.
    #[test]
    fn single_zero_bit_of_padding_is_rejected() {
        assert_eq!(
            decode_huffman_btree(&[0b1111_0000]),
            Err(HpackError::InvalidHuffmanPadding)
        );
    }

    // 'a' then 101: a 0 in the middle of the padding.
    #[test]
    fn padding_with_a_zero_in_the_middle_is_rejected() {
        assert_eq!(
            decode_huffman_btree(&[0b0001_1101]),
            Err(HpackError::InvalidHuffmanPadding)
        );
    }

    // 'a' then 011: a 0 at the start of the padding.
    #[test]
    fn padding_starting_with_a_zero_is_rejected() {
        assert_eq!(
            decode_huffman_btree(&[0b0001_1011]),
            Err(HpackError::InvalidHuffmanPadding)
        );
    }

    // 'a' then 11 bits set to 1: padding is at most 7 bits.
    #[test]
    fn padding_longer_than_7_bits_is_rejected() {
        assert_eq!(
            decode_huffman_btree(&[0b0001_1111, 0xff]),
            Err(HpackError::InvalidHuffmanPadding)
        );
    }

    // A whole byte of 1s is 8 bits of padding, even with nothing before it.
    #[test]
    fn a_byte_of_padding_alone_is_rejected() {
        assert_eq!(
            decode_huffman_btree(&[0xff]),
            Err(HpackError::InvalidHuffmanPadding)
        );
    }

    // 30 bits set to 1 = EOS: it must never appear in the data.
    #[test]
    fn eos_in_the_data_is_rejected() {
        assert_eq!(
            decode_huffman_btree(&[0xff, 0xff, 0xff, 0xff]),
            Err(HpackError::HuffmanEos)
        );
    }

    // --- The tree (arena: index 0 is the root, children are indices) ---

    // Follows `len` bits of `code` from the root, most significant first.
    // None if a child is missing on the way.
    fn walk(tree: &[Node], code: u32, len: usize) -> Option<&Node> {
        let mut node = &tree[0];
        for i in (0..len).rev() {
            let bit = ((code >> i) & 1) as usize;
            node = &tree[node.children[bit]?];
        }
        Some(node)
    }

    fn is_leaf(node: &Node) -> bool {
        node.children == [None, None]
    }

    // 257 leaves + 256 internal nodes: the code is complete (Kraft sum = 1).
    #[test]
    fn tree_has_513_nodes() {
        assert_eq!(build_btree().len(), 513);
    }

    #[test]
    fn tree_has_257_leaves() {
        let tree = build_btree();

        assert_eq!(tree.iter().filter(|node| is_leaf(node)).count(), 257);
    }

    // The root holds no symbol: no code is 0 bits long.
    #[test]
    fn root_is_an_internal_node() {
        let tree = build_btree();

        assert_eq!(tree[0].symbol, None);
        assert!(tree[0].children[0].is_some());
        assert!(tree[0].children[1].is_some());
    }

    // 'a' = 00011: left, left, left, right, right.
    #[test]
    fn a_is_at_the_end_of_00011() {
        let tree = build_btree();
        let node = walk(&tree, 0b00011, 5).unwrap();

        assert_eq!(node.symbol, Some(b'a' as u16));
        assert!(is_leaf(node));
    }

    // '3' = 011001: the example walked through in the explanation.
    #[test]
    fn three_is_at_the_end_of_011001() {
        let tree = build_btree();

        assert_eq!(walk(&tree, 0b011001, 6).unwrap().symbol, Some(b'3' as u16));
    }

    // EOS (256) does not fit in a u8: this is why symbols are u16.
    #[test]
    fn eos_is_at_the_end_of_thirty_ones() {
        let tree = build_btree();

        assert_eq!(walk(&tree, 0x3fff_ffff, 30).unwrap().symbol, Some(256));
    }

    // A prefix of a code leads to an internal node, never to a symbol.
    #[test]
    fn a_prefix_of_a_code_is_not_a_symbol() {
        let tree = build_btree();
        let node = walk(&tree, 0b0001, 4).unwrap(); // prefix of 'a' = 00011

        assert_eq!(node.symbol, None);
        assert!(!is_leaf(node));
    }

    #[test]
    fn every_code_leads_to_its_symbol() {
        let tree = build_btree();

        for (symbol, &(code, len)) in CODES.iter().enumerate() {
            let node =
                walk(&tree, code, len).unwrap_or_else(|| panic!("no path for symbol {symbol}"));

            assert_eq!(node.symbol, Some(symbol as u16), "symbol {symbol}");
            assert!(is_leaf(node), "symbol {symbol} is not on a leaf");
        }
    }

    // Prefix-free: a node is either a leaf with a symbol, or internal without one.
    #[test]
    fn symbols_are_only_on_leaves() {
        for (index, node) in build_btree().iter().enumerate() {
            assert_eq!(node.symbol.is_some(), is_leaf(node), "node {index}");
        }
    }

    // Complete code: an internal node never has a single child, so decoding
    // can never hit a dead end.
    #[test]
    fn internal_nodes_have_two_children() {
        for (index, node) in build_btree().iter().enumerate() {
            if !is_leaf(node) {
                assert!(
                    node.children[0].is_some() && node.children[1].is_some(),
                    "node {index} has a single child"
                );
            }
        }
    }

    // A tree, not a graph: every node except the root has exactly one parent,
    // and nothing points back to the root.
    #[test]
    fn every_node_except_the_root_has_one_parent() {
        let tree = build_btree();
        let mut parents = vec![0; tree.len()];

        for node in &tree {
            for child in node.children.iter().flatten() {
                parents[*child] += 1;
            }
        }

        assert_eq!(parents[0], 0);
        assert!(parents[1..].iter().all(|&count| count == 1));
    }

    // EOS followed by valid data: decoding must stop on the EOS leaf.
    #[test]
    fn tree_decoder_rejects_eos_before_more_data() {
        // EOS (30 ones) + 'a' (00011) = 35 bits, then 5 bits of padding.
        let input = [0xff, 0xff, 0xff, 0xfc, 0x7f];

        assert_eq!(decode_huffman_btree(&input), Err(HpackError::HuffmanEos));
    }
}
