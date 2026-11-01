pub mod decoder;
pub mod dynamic_table;
pub mod integer;
pub mod static_table;
pub mod string;

// RFC 9113 §4.3: any HPACK decoding error is a connection error (COMPRESSION_ERROR).
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HpackError {
    #[error("Truncated input")]
    Truncated,
    #[error("Integer overflow")]
    Overflow,
    #[error("Invalid table index")]
    InvalidIndex,
    #[error("Table size update above SETTINGS_HEADER_TABLE_SIZE")]
    InvalidTableSize,
    // RFC 7541 §4.2: a size update is only allowed at the start of a header block.
    #[error("Table size update after a header field")]
    UnexpectedTableSizeUpdate,
    // Temporary: Huffman decoding comes with sub-step 4.6.
    #[error("Huffman decoding not supported")]
    HuffmanUnsupported,
}
