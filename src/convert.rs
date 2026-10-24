use std::path::Path;

use crate::decode::decode_jpeg;
use crate::error::KfbError;
use crate::pyramid::Pyramid;
use crate::reader::KfbReader;
use crate::types::{AssociatedImageKind, DecodedAssociatedImage};

/// Read a `.kfb` file and write an OME-Zarr v0.4 hierarchy to `output`.
///
/// Tiles are decoded on demand as each resolution level is written. Each level becomes one
/// Zarr array with Blosc/Zstd-compressed chunks. Scale transforms use the MPP value from the file
/// header.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use kfb2zarr::convert_to_zarr;
///
/// convert_to_zarr(Path::new("slide.kfb"), Path::new("slide.ome.zarr"))?;
/// # Ok::<(), kfb2zarr::KfbError>(())
/// ```
///
/// # Errors
///
/// Returns [`KfbError::Io`], [`KfbError::InvalidMagic`] or [`KfbError::InvalidOffset`]
/// if the input file cannot be read, [`KfbError::JpegDecode`] if any tile fails to decode,
/// and [`KfbError::ZarrWrite`] if writing the output store fails.
pub fn convert_to_zarr(input: &Path, output: &Path) -> Result<(), KfbError> {
    convert_to_zarr_with_chunk_size(input, output, None)
}

/// Like [`convert_to_zarr`], but writes chunks of `chunk_size` pixels per side
/// (capped to each level's dimensions). `None` uses the source tile size.
///
/// # Errors
///
/// Same as [`convert_to_zarr`], plus [`KfbError::InvalidChunkSize`] if `chunk_size` is
/// not a positive multiple of the source tile size, or is larger than 4096. Nothing is
/// written in that case.
pub fn convert_to_zarr_with_chunk_size(
    input: &Path,
    output: &Path,
    chunk_size: Option<u64>,
) -> Result<(), KfbError> {
    let reader = KfbReader::open(input)?;
    let header = reader.header().clone();
    let pyramid = Pyramid::new(&header, reader.tiles());

    let decoded_associated = reader
        .associated_images()
        .iter()
        .filter(|img| img.kind() == AssociatedImageKind::Label)
        .map(|img| {
            let jpeg = reader.read_associated_bytes(img)?;
            let (pixels, width, height) = decode_jpeg(jpeg)?;
            Ok(DecodedAssociatedImage {
                kind: AssociatedImageKind::Label,
                pixels,
                width: width as u64,
                height: height as u64,
            })
        })
        .collect::<Result<Vec<_>, KfbError>>()?;

    crate::zarr::write_ome_zarr(
        output,
        &reader,
        &header,
        &pyramid,
        &decoded_associated,
        chunk_size,
    )
}
