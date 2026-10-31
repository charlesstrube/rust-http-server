pub mod integer;
pub mod string;

// RFC 9113 §4.3: any HPACK decoding error is a connection error (COMPRESSION_ERROR).
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HpackError {
    #[error("Truncated input")]
    Truncated,
    #[error("Integer overflow")]
    Overflow,
    // Temporary: Huffman decoding comes with sub-step 4.6.
    #[error("Huffman decoding not supported")]
    HuffmanUnsupported,
}
