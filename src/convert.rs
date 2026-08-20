use std::path::Path;

use crate::decode::decode_jpeg;
use crate::error::KfbError;
use crate::reader::KfbReader;
use crate::types::{AssociatedImageKind, DecodedAssociatedImage, TileInfo};

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
/// Returns [`KfbError::Io`] or [`KfbError::InvalidMagic`] if the input file cannot be
/// read, [`KfbError::JpegDecode`] if any tile fails to decode, and [`KfbError::ZarrWrite`]
/// if writing the output store fails.
pub fn convert_to_zarr(input: &Path, output: &Path) -> Result<(), KfbError> {
    let reader = KfbReader::open(input)?;
    let header = reader.header().clone();
    let zoom_level_count = header.zoom_levels().max(1) as usize;

    let mut by_level: Vec<Vec<TileInfo>> = vec![Vec::new(); zoom_level_count];
    for tile in reader.tiles() {
        let level = tile.zoom_level() as usize;
        if level < zoom_level_count && tile.data_length > 0 {
            by_level[level].push(tile.clone());
        }
    }

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

    crate::zarr::write_ome_zarr(output, &reader, &header, &by_level, &decoded_associated)
}
