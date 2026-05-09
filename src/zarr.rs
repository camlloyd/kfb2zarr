use std::fs;
use std::path::Path;
use std::sync::Arc;

use rayon::prelude::*;
use serde_json::json;
use zarrs::array::Array;
use zarrs::group::Group;
use zarrs::metadata::v2::{
    ArrayMetadataV2, DataTypeMetadataV2, FillValueMetadataV2, GroupMetadataV2, MetadataV2,
};
use zarrs::metadata::{ChunkKeySeparator, GroupMetadata};

use crate::error::KfbError;
use crate::types::{AssociatedImageKind, DecodedAssociatedImage, DecodedTile, KfbHeader};

fn zarr_err<E: std::fmt::Display>(e: E) -> KfbError {
    KfbError::ZarrWrite(e.to_string())
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn format_utc_timestamp(seconds: i64) -> Option<String> {
    if seconds <= 0 {
        return None;
    }

    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days)?;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn civil_from_days(days_since_epoch: i64) -> Option<(i32, u32, u32)> {
    let z = days_since_epoch.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    Some((
        year.try_into().ok()?,
        month.try_into().ok()?,
        day.try_into().ok()?,
    ))
}

fn ome_xml_color([r, g, b]: [u8; 3]) -> i32 {
    u32::from_be_bytes([r, g, b, 255]) as i32
}

fn write_ome_xml_metadata(
    output: &Path,
    header: &KfbHeader,
    size_x: u64,
    size_y: u64,
) -> Result<(), KfbError> {
    let metadata_dir = output.join("OME");
    fs::create_dir_all(&metadata_dir).map_err(zarr_err)?;

    let name = output.file_name().unwrap_or_default().to_string_lossy();
    let name = escape_xml_attr(&name);
    let physical_size = header.mpp();
    let magnification = header.scan_scale();
    let channel_count = header.channel_count();
    let acquisition_date = format_utc_timestamp(header.scan_time())
        .map(|timestamp| format!("<AcquisitionDate>{timestamp}</AcquisitionDate>"))
        .unwrap_or_default();
    let channels = if !header.channels().is_empty() {
        header
            .channels()
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let name = escape_xml_attr(&ch.name);
                let color = ome_xml_color(ch.color_rgb);
                format!(
                    r#"<Channel ID="Channel:0:{i}" Name="{name}" Color="{color}" SamplesPerPixel="1"/>"#
                )
            })
            .collect::<String>()
    } else {
        (0..channel_count)
            .map(|i| {
                let (name, color_rgb) = if header.is_fluorescence() {
                    let fluorescence_colors = [
                        ("0000FF", [0, 0, 255]),
                        ("00FF00", [0, 255, 0]),
                        ("FF0000", [255, 0, 0]),
                        ("FFFF00", [255, 255, 0]),
                        ("FF00FF", [255, 0, 255]),
                        ("00FFFF", [0, 255, 255]),
                    ];
                    let (_, color_rgb) = fluorescence_colors[i % fluorescence_colors.len()];
                    (format!("Channel {}", i + 1), color_rgb)
                } else {
                    let rgb_channels = [("R", [255, 0, 0]), ("G", [0, 255, 0]), ("B", [0, 0, 255])];
                    let (name, color_rgb) = rgb_channels.get(i).copied().unwrap_or(("Channel", [255, 255, 255]));
                    (name.to_string(), color_rgb)
                };
                let color = ome_xml_color(color_rgb);
                format!(
                    r#"<Channel ID="Channel:0:{i}" Name="{name}" Color="{color}" SamplesPerPixel="1"/>"#
                )
            })
            .collect::<String>()
    };

    let planes = if !header.channels().is_empty() {
        header
            .channels()
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                format!(
                    r#"<Plane TheZ="0" TheT="0" TheC="{i}" ExposureTime="{}" ExposureTimeUnit="ms"/>"#,
                    ch.exposure_ms
                )
            })
            .collect::<String>()
    } else {
        String::new()
    };

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><OME xmlns="http://www.openmicroscopy.org/Schemas/OME/2016-06" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" Creator="kfb2zarr {creator_version}" xsi:schemaLocation="http://www.openmicroscopy.org/Schemas/OME/2016-06 https://www.openmicroscopy.org/Schemas/OME/2016-06/ome.xsd"><Instrument ID="Instrument:0"><Objective ID="Objective:0:0" NominalMagnification="{magnification}"/></Instrument><Image ID="Image:0" Name="{name}">{acquisition_date}<InstrumentRef ID="Instrument:0"/><ObjectiveSettings ID="Objective:0:0"/><Pixels ID="Pixels:0" DimensionOrder="XYCZT" Type="uint8" SizeX="{size_x}" SizeY="{size_y}" SizeC="{channel_count}" SizeZ="1" SizeT="1" PhysicalSizeX="{physical_size}" PhysicalSizeXUnit="µm" PhysicalSizeY="{physical_size}" PhysicalSizeYUnit="µm">{channels}<MetadataOnly/>{planes}</Pixels></Image></OME>"#,
        creator_version = env!("CARGO_PKG_VERSION"),
        size_x = size_x,
        size_y = size_y,
        channel_count = channel_count,
        channels = channels,
        planes = planes,
    );

    fs::write(metadata_dir.join("METADATA.ome.xml"), xml).map_err(zarr_err)
}

fn level_dimensions(header: &KfbHeader, level_index: usize, tiles: &[DecodedTile]) -> (u64, u64) {
    let tile_bounds = tiles
        .iter()
        .fold(None::<(u64, u64)>, |bounds, (tile, _, _, _)| {
            let max_x = (tile.pos_x() + tile.width()).max(0) as u64;
            let max_y = (tile.pos_y() + tile.height()).max(0) as u64;
            Some(match bounds {
                Some((w, h)) => (w.max(max_x), h.max(max_y)),
                None => (max_x, max_y),
            })
        });
    let scale_factor = 1u64 << level_index;
    let header_bounds = (
        (header.base_width() as u64).div_ceil(scale_factor),
        (header.base_height() as u64).div_ceil(scale_factor),
    );
    match tile_bounds {
        Some((tile_w, tile_h)) => (header_bounds.0.max(tile_w), header_bounds.1.max(tile_h)),
        None => header_bounds,
    }
}

/// Reorder a tile's pixel data from interleaved HWC (JPEG output) to planar CHW
/// (OME-Zarr [c, y, x] convention).  Pads partial edge tiles to `dst_h × dst_w`.
fn hwc_to_chw_padded(
    hwc: &[u8],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
    fill_value: u8,
) -> Vec<u8> {
    let mut chw = vec![fill_value; 3 * dst_h * dst_w];
    for c in 0..3usize {
        for y in 0..src_h {
            for x in 0..src_w {
                chw[c * dst_h * dst_w + y * dst_w + x] = hwc[y * src_w * 3 + x * 3 + c];
            }
        }
    }
    chw
}

fn copy_transposed_luma_plane(
    chunk: &mut [u8],
    luma: &[u8],
    channel_index: usize,
    raw_w: usize,
    raw_h: usize,
    dst_w: usize,
    dst_h: usize,
) {
    let plane_start = channel_index * dst_h * dst_w;
    for raw_y in 0..raw_h {
        for raw_x in 0..raw_w {
            let dst_x = raw_y;
            let dst_y = raw_x;
            chunk[plane_start + dst_y * dst_w + dst_x] = luma[raw_y * raw_w + raw_x];
        }
    }
}

fn associated_image_name(kind: AssociatedImageKind) -> &'static str {
    match kind {
        AssociatedImageKind::Label => "label",
        AssociatedImageKind::Thumbnail => "thumbnail",
    }
}

fn rgb_omero_channels() -> serde_json::Value {
    let channel = |color: &str, label: &str| {
        json!({
            "active": true,
            "coefficient": 1,
            "color": color,
            "family": "linear",
            "inverted": false,
            "label": label,
            "window": {"end": 255, "max": 255, "min": 0, "start": 0}
        })
    };
    json!([
        channel("FF0000", "R"),
        channel("00FF00", "G"),
        channel("0000FF", "B"),
    ])
}

fn write_associated_image(
    store: Arc<zarrs::filesystem::FilesystemStore>,
    image: &DecodedAssociatedImage,
    compressor: MetadataV2,
) -> Result<(), KfbError> {
    let name = associated_image_name(image.kind);
    let group_path = format!("/associated/{name}");
    let array_path = format!("{group_path}/0");
    let axes = json!([
        {"name": "c", "type": "channel"},
        {"name": "y", "type": "space"},
        {"name": "x", "type": "space"}
    ]);
    let attrs = serde_json::Map::from_iter([
        (
            "multiscales".to_string(),
            json!([{
                "version": "0.4",
                "name": name,
                "axes": axes,
                "datasets": [{
                    "path": "0",
                    "coordinateTransformations": [
                        {"type": "scale", "scale": [1.0, 1.0, 1.0]}
                    ]
                }],
            }]),
        ),
        (
            "omero".to_string(),
            json!({
                "id": 1,
                "name": name,
                "version": "0.4",
                "channels": rgb_omero_channels(),
                "rdefs": {"defaultT": 0, "defaultZ": 0, "model": "color"}
            }),
        ),
    ]);

    let group_meta: GroupMetadata = GroupMetadataV2::new().with_attributes(attrs).into();
    Group::new_with_metadata(store.clone(), &group_path, group_meta)
        .map_err(zarr_err)?
        .store_metadata()
        .map_err(zarr_err)?;

    let array_meta = ArrayMetadataV2::new(
        vec![3, image.height, image.width],
        vec![3, image.height, image.width]
            .try_into()
            .map_err(zarr_err)?,
        DataTypeMetadataV2::Simple("|u1".into()),
        FillValueMetadataV2::Number(serde_json::Number::from(0u8)),
        Some(compressor),
        None,
    )
    .with_dimension_separator(ChunkKeySeparator::Slash)
    .with_attributes(serde_json::Map::from_iter([(
        "_ARRAY_DIMENSIONS".to_string(),
        json!(["c", "y", "x"]),
    )]));

    let array =
        Array::new_with_metadata(store, &array_path, array_meta.into()).map_err(zarr_err)?;
    array.store_metadata().map_err(zarr_err)?;
    let chw = hwc_to_chw_padded(
        &image.pixels,
        image.width as usize,
        image.height as usize,
        image.width as usize,
        image.height as usize,
        0,
    );
    array
        .store_chunk_elements::<u8>(&[0, 0, 0], &chw)
        .map_err(zarr_err)
}

fn write_fluorescence_level(
    array: &Array<zarrs::filesystem::FilesystemStore>,
    tiles: &[DecodedTile],
    channel_count: usize,
    tile_size: u64,
    fill_value: u8,
) -> Result<(), KfbError> {
    let ts = tile_size as usize;
    let mut chunks = std::collections::BTreeMap::<(u64, u64), Vec<u8>>::new();
    for (tile_info, pixels, tile_w, tile_h) in tiles {
        let cy = tile_info.pos_y() as u64 / tile_size;
        let cx = tile_info.pos_x() as u64 / tile_size;
        let chunk = chunks
            .entry((cy, cx))
            .or_insert_with(|| vec![fill_value; channel_count * ts * ts]);
        copy_transposed_luma_plane(
            chunk,
            pixels,
            tile_info.channel_index(),
            *tile_h as usize,
            *tile_w as usize,
            ts,
            ts,
        );
    }
    chunks.into_par_iter().try_for_each(|((cy, cx), chunk)| {
        array
            .store_chunk_elements::<u8>(&[0, cy, cx], &chunk)
            .map_err(zarr_err)
    })
}

fn write_brightfield_level(
    array: &Array<zarrs::filesystem::FilesystemStore>,
    tiles: &[DecodedTile],
    tile_size: u64,
    fill_value: u8,
) -> Result<(), KfbError> {
    tiles
        .par_iter()
        .try_for_each(|(tile_info, pixels, tile_w, tile_h)| {
            let cy = tile_info.pos_y() as u64 / tile_size;
            let cx = tile_info.pos_x() as u64 / tile_size;
            let ts = tile_size as usize;
            let chw = hwc_to_chw_padded(
                pixels,
                *tile_w as usize,
                *tile_h as usize,
                ts,
                ts,
                fill_value,
            );
            array
                .store_chunk_elements::<u8>(&[0, cy, cx], &chw)
                .map_err(zarr_err)
        })
}

/// Write decoded tiles as an OME-Zarr 0.4 (Zarr v2) hierarchy at `output`.
///
/// Arrays use `[c, y, x]` axis order with Blosc/LZ4-compressed uint8 chunks.
/// The root group has OME-Zarr 0.4 multiscales metadata with per-level
/// scale transforms derived from `header.mpp()`.
///
pub(crate) fn write_ome_zarr(
    output: &Path,
    header: &KfbHeader,
    tiles_by_level: &[Vec<DecodedTile>],
    associated_images: &[DecodedAssociatedImage],
) -> Result<(), KfbError> {
    let store = Arc::new(zarrs::filesystem::FilesystemStore::new(output).map_err(zarr_err)?);

    let tile_size = tiles_by_level
        .first()
        .and_then(|t| t.first())
        .map(|(_, _, w, _)| *w)
        .unwrap_or(header.tile_size() as u64);
    let mpp = header.mpp();
    let channel_count = header.channel_count();
    let num_levels = tiles_by_level.len();
    let level_dimensions: Vec<_> = tiles_by_level
        .iter()
        .enumerate()
        .map(|(i, tiles)| level_dimensions(header, i, tiles))
        .collect();
    let (base_level_w, base_level_h) = level_dimensions
        .first()
        .copied()
        .unwrap_or((header.base_width() as u64, header.base_height() as u64));

    let axes = json!([
        {"name": "c", "type": "channel"},
        {"name": "y", "type": "space",   "unit": "micrometer"},
        {"name": "x", "type": "space",   "unit": "micrometer"}
    ]);
    let mut datasets = Vec::with_capacity(num_levels);
    for i in 0..num_levels {
        let scale_factor = (1u64 << i) as f64;
        datasets.push(json!({
            "path": i.to_string(),
            "coordinateTransformations": [
                {"type": "scale", "scale": [1.0, mpp * scale_factor, mpp * scale_factor]}
            ]
        }));
    }
    let channel = |color: &str, label: &str, fill: u8| {
        json!({
            "active": true,
            "coefficient": 1,
            "color": color,
            "family": "linear",
            "inverted": false,
            "label": label,
            "window": {"end": 255, "max": 255, "min": 0, "start": fill}
        })
    };
    let channels: Vec<_> = if !header.channels().is_empty() {
        header
            .channels()
            .iter()
            .map(|ch| {
                let hex = format!(
                    "{:02X}{:02X}{:02X}",
                    ch.color_rgb[0], ch.color_rgb[1], ch.color_rgb[2]
                );
                channel(&hex, &ch.name, 0)
            })
            .collect()
    } else if header.is_fluorescence() {
        let fluorescence_colors = ["0000FF", "00FF00", "FF0000", "FFFF00", "FF00FF", "00FFFF"];
        (0..channel_count)
            .map(|i| {
                channel(
                    fluorescence_colors[i % fluorescence_colors.len()],
                    &format!("Channel {}", i + 1),
                    0,
                )
            })
            .collect()
    } else {
        vec![
            channel("FF0000", "R", 0),
            channel("00FF00", "G", 0),
            channel("0000FF", "B", 0),
        ]
    };
    let omero = json!({
        "id": 1,
        "name": output.file_name().unwrap_or_default().to_string_lossy(),
        "version": "0.4",
        "channels": channels,
        "rdefs": {"defaultT": 0, "defaultZ": 0, "model": "color"}
    });

    let associated_attrs: Vec<_> = associated_images
        .iter()
        .map(|image| {
            let name = associated_image_name(image.kind);
            json!({"kind": name, "path": format!("associated/{name}")})
        })
        .collect();

    let mut multiscales_json = json!({
        "multiscales": [{
            "version": "0.4",
            "name": output.file_stem().unwrap_or_default().to_string_lossy(),
            "axes": axes,
            "datasets": datasets,
        }],
        "omero": omero,
    });
    if !associated_attrs.is_empty() {
        multiscales_json["associated_images"] = json!(associated_attrs);
    }
    let serde_json::Value::Object(attrs) = multiscales_json else {
        unreachable!()
    };

    let compressor: MetadataV2 = serde_json::from_value(json!({
        "id": "blosc",
        "cname": "lz4",
        "clevel": 5,
        "shuffle": 1,
        "blocksize": 0
    }))
    .map_err(zarr_err)?;

    let group_meta: GroupMetadata = GroupMetadataV2::new().with_attributes(attrs).into();
    Group::new_with_metadata(store.clone(), "/", group_meta)
        .map_err(zarr_err)?
        .store_metadata()
        .map_err(zarr_err)?;
    write_ome_xml_metadata(output, header, base_level_w, base_level_h)?;

    if !associated_images.is_empty() {
        let names: Vec<_> = associated_images
            .iter()
            .map(|image| associated_image_name(image.kind))
            .collect();
        let attrs = serde_json::Map::from_iter([("images".to_string(), json!(names))]);
        let group_meta: GroupMetadata = GroupMetadataV2::new().with_attributes(attrs).into();
        Group::new_with_metadata(store.clone(), "/associated", group_meta)
            .map_err(zarr_err)?
            .store_metadata()
            .map_err(zarr_err)?;
        for image in associated_images {
            write_associated_image(store.clone(), image, compressor.clone())?;
        }
    }

    for (i, tiles) in tiles_by_level.iter().enumerate() {
        let (level_w, level_h) = level_dimensions[i];
        let fill_value = if header.is_fluorescence() { 0u8 } else { 255u8 };

        // 3D CYX: C=channel_count, Y=level_h, X=level_w.
        // A single chunk spans all channels for a given (y, x) tile.
        let array_meta = ArrayMetadataV2::new(
            vec![channel_count as u64, level_h, level_w],
            vec![channel_count as u64, tile_size, tile_size]
                .try_into()
                .map_err(zarr_err)?,
            DataTypeMetadataV2::Simple("|u1".into()),
            FillValueMetadataV2::Number(serde_json::Number::from(fill_value)),
            Some(compressor.clone()),
            None,
        )
        .with_dimension_separator(ChunkKeySeparator::Slash)
        .with_attributes(serde_json::Map::from_iter([(
            "_ARRAY_DIMENSIONS".to_string(),
            json!(["c", "y", "x"]),
        )]));

        let array = Array::new_with_metadata(store.clone(), &format!("/{i}"), array_meta.into())
            .map_err(zarr_err)?;
        array.store_metadata().map_err(zarr_err)?;

        if header.is_fluorescence() {
            write_fluorescence_level(&array, tiles, channel_count, tile_size, fill_value)?;
        } else {
            write_brightfield_level(&array, tiles, tile_size, fill_value)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ChannelMetadata, KfbFormat, KfbHeader, KfbHeaderFields, TileInfo, TileInfoFields,
    };
    use tempfile::TempDir;

    fn header(
        tile_count: i32,
        base_width: i32,
        base_height: i32,
        mpp: f64,
        zoom_levels: i32,
    ) -> KfbHeader {
        header_with_scan_time(tile_count, base_width, base_height, mpp, zoom_levels, 0)
    }

    fn header_with_scan_time(
        tile_count: i32,
        base_width: i32,
        base_height: i32,
        mpp: f64,
        zoom_levels: i32,
        scan_time: i64,
    ) -> KfbHeader {
        KfbHeader::new(KfbHeaderFields {
            format: KfbFormat::Brightfield,
            tile_count,
            base_width,
            base_height,
            scan_scale: 20,
            spend_time: 0,
            scan_time,
            image_cap_res: mpp,
            tile_size: 256,
            channel_count: 3,
            zoom_levels,
            channels: vec![],
        })
    }

    fn fluorescence_header(channel_count: usize) -> KfbHeader {
        KfbHeader::new(KfbHeaderFields {
            format: KfbFormat::Fluorescence,
            tile_count: channel_count as i32,
            base_width: 512,
            base_height: 512,
            scan_scale: 40,
            spend_time: 0,
            scan_time: 0,
            image_cap_res: 0.25,
            tile_size: 512,
            channel_count,
            zoom_levels: 1,
            channels: vec![],
        })
    }

    fn make_single_tile_zarr() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("test.zarr");
        let header = header(1, 256, 256, 0.25, 1);
        let pixels = vec![128u8; 256 * 256 * 3];
        let tile = TileInfo::new(0, 0, 256, 256, 0, 0, 0);
        write_ome_zarr(
            &output,
            &header,
            &[vec![(tile, pixels, 256u64, 256u64)]],
            &[],
        )
        .unwrap();
        (dir, output)
    }

    fn make_two_level_zarr() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("test.zarr");
        let header = header(2, 512, 512, 0.25, 2);
        let pixels = vec![128u8; 256 * 256 * 3];
        let tile = TileInfo::new(0, 0, 256, 256, 0, 0, 0);
        write_ome_zarr(
            &output,
            &header,
            &[
                vec![(tile.clone(), pixels.clone(), 256u64, 256u64)],
                vec![(tile, pixels, 256u64, 256u64)],
            ],
            &[],
        )
        .unwrap();
        (dir, output)
    }

    fn read_json(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn array_fill_value_is_white() {
        let (_dir, out) = make_single_tile_zarr();
        assert_eq!(read_json(&out.join("0/.zarray"))["fill_value"], 255);
    }

    #[test]
    fn array_shape_is_cyx() {
        let (_dir, out) = make_single_tile_zarr();
        let z = read_json(&out.join("0/.zarray"));
        assert_eq!(z["shape"], json!([3, 256, 256]));
        assert_eq!(z["chunks"], json!([3, 256, 256]));
    }

    #[test]
    fn chunks_use_ngff_0_4_slash_separator() {
        let (_dir, out) = make_single_tile_zarr();
        let z = read_json(&out.join("0/.zarray"));
        assert_eq!(z["dimension_separator"], "/");
        assert!(
            out.join("0/0/0/0").exists(),
            "chunk should use the OME-NGFF 0.4 slash separator"
        );
        assert!(
            !out.join("0/0.0.0").exists(),
            "chunk should not be stored with dotted chunk keys"
        );
    }

    #[test]
    fn array_dtype_is_uint8() {
        let (_dir, out) = make_single_tile_zarr();
        let z = read_json(&out.join("0/.zarray"));
        assert_eq!(z["dtype"], "|u1");
    }

    #[test]
    fn array_uses_blosc_lz4_compressor() {
        let (_dir, out) = make_single_tile_zarr();
        let z = read_json(&out.join("0/.zarray"));
        assert_eq!(z["compressor"]["id"], "blosc");
        assert_eq!(z["compressor"]["cname"], "lz4");
        assert_eq!(z["compressor"]["clevel"], 5);
        assert_eq!(z["compressor"]["shuffle"], 1);
        assert_eq!(z["compressor"]["blocksize"], 0);
    }

    #[test]
    fn array_zattrs_record_cyx_dimensions() {
        let (_dir, out) = make_single_tile_zarr();
        let attrs = read_json(&out.join("0/.zattrs"));
        assert_eq!(attrs["_ARRAY_DIMENSIONS"], json!(["c", "y", "x"]));
    }

    #[test]
    fn slide_label_is_written_as_associated_image() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("test.zarr");
        let header = header(1, 256, 256, 0.25, 1);
        let pixels = vec![128u8; 256 * 256 * 3];
        let tile = TileInfo::new(0, 0, 256, 256, 0, 0, 0);
        let label = DecodedAssociatedImage {
            kind: AssociatedImageKind::Label,
            pixels: vec![255, 0, 0, 0, 255, 0],
            width: 2,
            height: 1,
        };

        write_ome_zarr(
            &output,
            &header,
            &[vec![(tile, pixels, 256u64, 256u64)]],
            &[label],
        )
        .unwrap();

        let root_attrs = read_json(&output.join(".zattrs"));
        assert_eq!(
            root_attrs["associated_images"],
            json!([{"kind": "label", "path": "associated/label"}])
        );
        assert_eq!(
            read_json(&output.join("associated/.zattrs"))["images"],
            json!(["label"])
        );
        assert_eq!(
            read_json(&output.join("associated/label/.zattrs"))["multiscales"][0]["datasets"][0]["path"],
            "0"
        );
        assert_eq!(
            read_json(&output.join("associated/label/0/.zarray"))["shape"],
            json!([3, 1, 2])
        );
        assert!(
            output.join("associated/label/0/0/0/0").exists(),
            "associated label chunk should be written"
        );
    }

    #[test]
    fn multiscales_version_is_0_4() {
        let (_dir, out) = make_single_tile_zarr();
        assert!(out.join(".zgroup").exists(), ".zgroup not found");
        assert_eq!(
            read_json(&out.join(".zattrs"))["multiscales"][0]["version"],
            "0.4"
        );
    }

    #[test]
    fn axes_are_cyx() {
        let (_dir, out) = make_single_tile_zarr();
        let axes = read_json(&out.join(".zattrs"))["multiscales"][0]["axes"].clone();
        let names: Vec<&str> = axes
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["c", "y", "x"]);
    }

    #[test]
    fn datasets_match_levels_and_scale_vectors() {
        let (_dir, out) = make_two_level_zarr();
        let attrs = read_json(&out.join(".zattrs"));
        let datasets = attrs["multiscales"][0]["datasets"].as_array().unwrap();
        assert_eq!(datasets.len(), 2);
        assert_eq!(datasets[0]["path"], "0");
        assert_eq!(datasets[1]["path"], "1");
        assert_eq!(
            datasets[0]["coordinateTransformations"][0]["scale"],
            json!([1.0, 0.25, 0.25])
        );
        assert_eq!(
            datasets[1]["coordinateTransformations"][0]["scale"],
            json!([1.0, 0.5, 0.5])
        );
    }

    #[test]
    fn each_pyramid_level_halves_resolution() {
        let (_dir, out) = make_two_level_zarr();
        assert_eq!(
            read_json(&out.join("0/.zarray"))["shape"],
            json!([3, 512, 512])
        );
        assert_eq!(
            read_json(&out.join("1/.zarray"))["shape"],
            json!([3, 256, 256])
        );
    }

    #[test]
    fn shape_uses_tile_bounds_when_header_is_short() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("test.zarr");
        let header = header(1, 37764, 25650, 0.247488, 1);
        let pixels = vec![128u8; 132 * 55 * 3];
        let tile = TileInfo::new(37632, 25600, 132, 55, 0, 0, 0);
        write_ome_zarr(
            &output,
            &header,
            &[vec![(tile, pixels, 132u64, 55u64)]],
            &[],
        )
        .unwrap();

        assert_eq!(
            read_json(&output.join("0/.zarray"))["shape"],
            json!([3, 25655, 37764])
        );
        let xml = std::fs::read_to_string(output.join("OME/METADATA.ome.xml")).unwrap();
        assert!(xml.contains(r#"SizeX="37764""#));
        assert!(xml.contains(r#"SizeY="25655""#));
    }

    #[test]
    fn ome_xml_records_acquisition_date_as_utc() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("test.zarr");
        let header = header_with_scan_time(1, 256, 256, 0.25, 1, 1_773_884_060);
        let pixels = vec![128u8; 256 * 256 * 3];
        let tile = TileInfo::new(0, 0, 256, 256, 0, 0, 0);
        write_ome_zarr(
            &output,
            &header,
            &[vec![(tile, pixels, 256u64, 256u64)]],
            &[],
        )
        .unwrap();

        let xml = std::fs::read_to_string(output.join("OME/METADATA.ome.xml")).unwrap();
        assert!(xml.contains("<AcquisitionDate>2026-03-19T01:34:20Z</AcquisitionDate>"));
    }

    #[test]
    fn utc_timestamp_formatter_handles_leap_days_and_epoch_boundary() {
        assert_eq!(
            format_utc_timestamp(1),
            Some("1970-01-01T00:00:01Z".to_string())
        );
        assert_eq!(
            format_utc_timestamp(1_583_020_799),
            Some("2020-02-29T23:59:59Z".to_string())
        );
        assert_eq!(format_utc_timestamp(0), None);
    }

    #[test]
    fn ome_xml_is_single_line() {
        let (_dir, out) = make_single_tile_zarr();
        let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
        assert!(!xml.contains('\n'));
        assert!(xml.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?><OME "#));
    }

    #[test]
    fn ome_xml_records_creator_and_metadata_only() {
        let (_dir, out) = make_single_tile_zarr();
        let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
        let expected = format!(r#"Creator="kfb2zarr {}""#, env!("CARGO_PKG_VERSION"));
        assert!(xml.contains(&expected), "missing {expected:?} in {xml}");
        assert!(xml.contains("<MetadataOnly/>"));
    }

    #[test]
    fn brightfield_omero_defaults_to_three_rgb_channels() {
        let (_dir, out) = make_single_tile_zarr();
        let channels = read_json(&out.join(".zattrs"))["omero"]["channels"].clone();
        assert_eq!(channels.as_array().unwrap().len(), 3);
        assert_eq!(channels[0]["color"], "FF0000");
        assert_eq!(channels[1]["color"], "00FF00");
        assert_eq!(channels[2]["color"], "0000FF");
    }

    #[test]
    fn fluorescence_zarr_uses_source_channel_count() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("test.zarr");
        let header = fluorescence_header(6);
        let tiles = (0..6usize)
            .map(|channel| {
                let tile = TileInfo::from_fields(TileInfoFields {
                    pos_x: 0,
                    pos_y: 0,
                    width: 512,
                    height: 512,
                    channel_index: channel,
                    zoom_level: 0,
                    data_offset: 0,
                    data_length: 0,
                });
                (tile, vec![channel as u8; 512 * 512], 512u64, 512u64)
            })
            .collect::<Vec<_>>();

        write_ome_zarr(&output, &header, &[tiles], &[]).unwrap();

        let z = read_json(&output.join("0/.zarray"));
        assert_eq!(z["shape"], json!([6, 512, 512]));
        assert_eq!(z["chunks"], json!([6, 512, 512]));
        assert_eq!(z["fill_value"], 0);
        let channels = read_json(&output.join(".zattrs"))["omero"]["channels"].clone();
        assert_eq!(channels.as_array().unwrap().len(), 6);
        let xml = std::fs::read_to_string(output.join("OME/METADATA.ome.xml")).unwrap();
        assert!(xml.contains(r#"SizeC="6""#));
        assert!(xml.contains(r#"Name="Channel 6""#));
    }

    #[test]
    fn ome_xml_records_physical_size_and_magnification() {
        let (_dir, out) = make_single_tile_zarr();
        let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
        assert!(xml.contains(r#"NominalMagnification="20""#));
        assert!(xml.contains(r#"PhysicalSizeX="0.25""#));
        assert!(xml.contains(r#"PhysicalSizeY="0.25""#));
        assert!(xml.contains(r#"PhysicalSizeXUnit="µm""#));
        assert!(xml.contains(r#"PhysicalSizeYUnit="µm""#));
        assert!(xml.contains(r#"SizeX="256""#));
        assert!(xml.contains(r#"SizeY="256""#));
    }

    fn make_fluorescence_zarr_with_channels() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("test.zarr");
        let header = fluorescence_header_with_channels();
        let tiles: Vec<_> = (0..2usize)
            .map(|ch| {
                let tile = TileInfo::from_fields(TileInfoFields {
                    pos_x: 0,
                    pos_y: 0,
                    width: 512,
                    height: 512,
                    channel_index: ch,
                    zoom_level: 0,
                    data_offset: 0,
                    data_length: 0,
                });
                (tile, vec![0u8; 512 * 512], 512u64, 512u64)
            })
            .collect();
        write_ome_zarr(&output, &header, &[tiles], &[]).unwrap();
        (dir, output)
    }

    fn fluorescence_header_with_channels() -> KfbHeader {
        KfbHeader::new(KfbHeaderFields {
            format: KfbFormat::Fluorescence,
            tile_count: 2,
            base_width: 512,
            base_height: 512,
            scan_scale: 40,
            spend_time: 0,
            scan_time: 0,
            image_cap_res: 0.25,
            tile_size: 512,
            channel_count: 2,
            zoom_levels: 1,
            channels: vec![
                ChannelMetadata {
                    name: "DAPI".to_string(),
                    color_rgb: [0, 0, 255],
                    exposure_ms: 10.0,
                },
                ChannelMetadata {
                    name: "FITC".to_string(),
                    color_rgb: [0, 255, 0],
                    exposure_ms: 30.0,
                },
            ],
        })
    }

    #[test]
    fn fluorescence_omero_uses_channel_names_and_colors() {
        let (_dir, out) = make_fluorescence_zarr_with_channels();
        let channels = read_json(&out.join(".zattrs"))["omero"]["channels"].clone();
        assert_eq!(channels[0]["label"], "DAPI");
        assert_eq!(channels[0]["color"], "0000FF");
        assert_eq!(channels[1]["label"], "FITC");
        assert_eq!(channels[1]["color"], "00FF00");
    }

    #[test]
    fn fluorescence_ome_xml_uses_channel_names() {
        let (_dir, out) = make_fluorescence_zarr_with_channels();
        let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
        assert!(
            xml.contains(r#"Name="DAPI""#),
            "expected DAPI channel name in XML"
        );
        assert!(
            xml.contains(r#"Name="FITC""#),
            "expected FITC channel name in XML"
        );
        assert!(
            !xml.contains(r#"Name="Channel 1""#),
            "should not contain generic name"
        );
    }

    #[test]
    fn fluorescence_ome_xml_uses_channel_colors() {
        let (_dir, out) = make_fluorescence_zarr_with_channels();
        let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
        assert!(
            xml.contains(r#"Name="DAPI" Color="65535""#),
            "expected DAPI to be blue RGBA in OME-XML"
        );
        assert!(
            xml.contains(r#"Name="FITC" Color="16711935""#),
            "expected FITC to be green RGBA in OME-XML"
        );
    }

    #[test]
    fn fluorescence_ome_xml_has_plane_exposure_times() {
        let (_dir, out) = make_fluorescence_zarr_with_channels();
        let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
        assert!(
            xml.contains(r#"ExposureTime="10" ExposureTimeUnit="ms""#)
                || xml.contains(r#"ExposureTime="10.0" ExposureTimeUnit="ms""#),
            "expected DAPI exposure time in XML"
        );
        assert!(
            xml.contains(r#"ExposureTime="30" ExposureTimeUnit="ms""#)
                || xml.contains(r#"ExposureTime="30.0" ExposureTimeUnit="ms""#),
            "expected FITC exposure time in XML"
        );
        assert!(xml.contains(r#"TheC="0""#), "expected Plane for channel 0");
        assert!(xml.contains(r#"TheC="1""#), "expected Plane for channel 1");
    }

    fn make_array(
        dir: &TempDir,
        channel_count: usize,
        size: usize,
        fill: u8,
    ) -> Array<zarrs::filesystem::FilesystemStore> {
        let store = Arc::new(zarrs::filesystem::FilesystemStore::new(dir.path()).unwrap());
        let meta = ArrayMetadataV2::new(
            vec![channel_count as u64, size as u64, size as u64],
            vec![channel_count as u64, size as u64, size as u64]
                .try_into()
                .unwrap(),
            DataTypeMetadataV2::Simple("|u1".into()),
            FillValueMetadataV2::Number(serde_json::Number::from(fill)),
            None,
            None,
        )
        .with_dimension_separator(ChunkKeySeparator::Slash);
        let array = Array::new_with_metadata(store, "/0", meta.into()).unwrap();
        array.store_metadata().unwrap();
        array
    }

    #[test]
    fn brightfield_level_writes_correct_pixel_data() {
        let dir = TempDir::new().unwrap();
        let array = make_array(&dir, 3, 4, 255);
        let pixels = vec![
            10u8, 20, 30, 11, 21, 31, 12, 22, 32, 13, 23, 33, 14, 24, 34, 15, 25, 35, 16, 26, 36,
            17, 27, 37, 18, 28, 38, 19, 29, 39, 10, 20, 30, 11, 21, 31, 12, 22, 32, 13, 23, 33, 14,
            24, 34, 15, 25, 35,
        ]; // 4×4 HWC RGB
        let tile = TileInfo::new(0, 0, 4, 4, 0, 0, 0);
        write_brightfield_level(&array, &[(tile, pixels, 4, 4)], 4, 255).unwrap();

        let chunk: Vec<u8> = array.retrieve_chunk_elements(&[0, 0, 0]).unwrap();
        // CHW: first 16 bytes are the red channel
        assert_eq!(chunk[0], 10, "R[0,0]");
        assert_eq!(chunk[16], 20, "G[0,0]");
        assert_eq!(chunk[32], 30, "B[0,0]");
    }

    #[test]
    fn brightfield_level_pads_partial_tiles_with_white() {
        let dir = TempDir::new().unwrap();
        let array = make_array(&dir, 3, 4, 255);
        let pixels = vec![10u8, 20, 30, 11, 21, 31, 12, 22, 32, 13, 23, 33]; // 2×2 HWC RGB
        let tile = TileInfo::new(0, 0, 2, 2, 0, 0, 0);
        write_brightfield_level(&array, &[(tile, pixels, 2, 2)], 4, 255).unwrap();

        let chunk: Vec<u8> = array.retrieve_chunk_elements(&[0, 0, 0]).unwrap();
        assert_eq!(chunk[0], 10, "R[0,0]");
        assert_eq!(chunk[1], 11, "R[0,1]");
        assert_eq!(chunk[4], 12, "R[1,0]");
        assert_eq!(chunk[5], 13, "R[1,1]");
        assert_eq!(chunk[2], 255, "R padding");
        assert_eq!(chunk[18], 255, "G padding");
        assert_eq!(chunk[34], 255, "B padding");
    }

    #[test]
    fn fluorescence_level_assembles_channels_into_chunk() {
        let dir = TempDir::new().unwrap();
        let array = make_array(&dir, 2, 4, 0);
        let ch0_pixels = vec![42u8; 16];
        let ch1_pixels = vec![99u8; 16];
        let t0 = TileInfo::from_fields(TileInfoFields {
            pos_x: 0,
            pos_y: 0,
            width: 4,
            height: 4,
            channel_index: 0,
            zoom_level: 0,
            data_offset: 0,
            data_length: 0,
        });
        let t1 = TileInfo::from_fields(TileInfoFields {
            pos_x: 0,
            pos_y: 0,
            width: 4,
            height: 4,
            channel_index: 1,
            zoom_level: 0,
            data_offset: 0,
            data_length: 0,
        });
        write_fluorescence_level(
            &array,
            &[(t0, ch0_pixels, 4, 4), (t1, ch1_pixels, 4, 4)],
            2,
            4,
            0,
        )
        .unwrap();

        let chunk: Vec<u8> = array.retrieve_chunk_elements(&[0, 0, 0]).unwrap();
        assert!(chunk[..16].iter().all(|&b| b == 42), "channel 0 pixels");
        assert!(chunk[16..].iter().all(|&b| b == 99), "channel 1 pixels");
    }

    #[test]
    fn fluorescence_level_transposes_raw_luma_into_chunk() {
        let dir = TempDir::new().unwrap();
        let array = make_array(&dir, 1, 4, 0);
        let tile = TileInfo::from_fields(TileInfoFields {
            pos_x: 0,
            pos_y: 0,
            width: 3,
            height: 2,
            channel_index: 0,
            zoom_level: 0,
            data_offset: 0,
            data_length: 0,
        });
        let raw_luma = vec![1u8, 2, 3, 4, 5, 6];

        write_fluorescence_level(&array, &[(tile, raw_luma, 3, 2)], 1, 4, 0).unwrap();

        let chunk: Vec<u8> = array.retrieve_chunk_elements(&[0, 0, 0]).unwrap();
        assert_eq!(&chunk[0..4], &[1, 3, 5, 0]);
        assert_eq!(&chunk[4..8], &[2, 4, 6, 0]);
    }
}
