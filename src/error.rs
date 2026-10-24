use thiserror::Error;

/// Errors that can be returned by this crate.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum KfbError {
    /// An underlying I/O operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// The file does not contain the expected KFB magic bytes at a given offset.
    #[error("invalid KFB magic bytes at offset {offset}: expected {expected:?}, got {actual:?}")]
    InvalidMagic {
        /// Byte offset in the file where the magic bytes were read.
        offset: u64,
        /// Magic bytes the reader expected at `offset`.
        expected: [u8; 4],
        /// Bytes found at `offset`.
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
    InvalidOffset {
        /// Byte offset where the data range starts.
        offset: u64,
        /// Length of the file in bytes.
        file_len: u64,
    },
    /// The requested chunk size is not a positive multiple of the source tile size, or is
    /// larger than 4096.
    #[error(
        "chunk size {chunk_size} must be a positive multiple of the tile size {tile_size}, at most {}",
        crate::zarr::MAX_CHUNK_SIZE
    )]
    InvalidChunkSize {
        /// Chunk size requested by the caller, in pixels per side.
        chunk_size: u64,
        /// Tile size of the source file, in pixels per side.
        tile_size: u64,
    },
    /// Writing the Zarr store failed with the given diagnostic message.
    #[error("Zarr write error: {0}")]
    ZarrWrite(String),
}
