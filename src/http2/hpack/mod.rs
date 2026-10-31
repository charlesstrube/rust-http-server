pub mod integer;

// RFC 9113 §4.3: any HPACK decoding error is a connection error (COMPRESSION_ERROR).
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HpackError {
    #[error("Truncated input")]
    Truncated,
    #[error("Integer overflow")]
    Overflow,
}
