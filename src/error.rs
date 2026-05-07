use thiserror::Error;

/// Errors that can be returned by this crate.
#[derive(Debug, Error)]
pub enum KfbError {
    /// An underlying I/O operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// The file does not contain the expected KFB magic bytes at a given offset.
    #[error("invalid KFB magic bytes at offset {offset}: expected {expected:?}, got {actual:?}")]
    InvalidMagic {
        offset: u64,
        expected: [u8; 4],
        actual: [u8; 4],
    },
    /// JPEG decoding failed with the given diagnostic message.
    #[error("JPEG decode error: {0}")]
    JpegDecode(String),
    /// A required section marker was not found while scanning the file.
    #[error("section not found: {0}")]
    SectionNotFound(&'static str),
    /// A tile or associated-image data range extends beyond the end of the file.
    #[error("data offset {offset} + length exceeds file size {file_len}")]
    InvalidOffset { offset: u64, file_len: u64 },
    /// Writing the Zarr store failed with the given diagnostic message.
    #[error("Zarr write error: {0}")]
    ZarrWrite(String),
}
