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

use crate::decode::{decode_jpeg, decode_jpeg_luma};
use crate::error::KfbError;
use crate::reader::KfbReader;
use crate::types::{AssociatedImageKind, DecodedAssociatedImage, KfbHeader, TileInfo};

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

fn tile_level_dimensions(header: &KfbHeader, level_index: usize, tiles: &[TileInfo]) -> (u64, u64) {
    let tile_bounds = tiles.iter().fold(None::<(u64, u64)>, |bounds, tile| {
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
/// (OME-Zarr [c, y, x] convention).  Pads partial edge tiles to `dst_h × dst_w`
/// and clips any decoded padding beyond the chunk boundary.
fn hwc_to_chw_padded(
    hwc: &[u8],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
    fill_value: u8,
) -> Vec<u8> {
    let mut chw = vec![fill_value; 3 * dst_h * dst_w];
    let copy_w = src_w.min(dst_w);
    let copy_h = src_h.min(dst_h);
    for c in 0..3usize {
        for y in 0..copy_h {
            for x in 0..copy_w {
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
    let copy_raw_y = raw_h.min(dst_w);
    let copy_raw_x = raw_w.min(dst_h);
    for raw_y in 0..copy_raw_y {
        for raw_x in 0..copy_raw_x {
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

fn group_tiles_by_spatial_chunk(
    tiles: &[TileInfo],
    tile_size: u64,
) -> std::collections::BTreeMap<(u64, u64), Vec<&TileInfo>> {
    let mut chunks = std::collections::BTreeMap::<(u64, u64), Vec<&TileInfo>>::new();
    for tile in tiles {
        let cy = tile.pos_y() as u64 / tile_size;
        let cx = tile.pos_x() as u64 / tile_size;
        chunks.entry((cy, cx)).or_default().push(tile);
    }
    chunks
}

fn write_fluorescence_level(
    array: &Array<zarrs::filesystem::FilesystemStore>,
    reader: &KfbReader,
    tiles: &[TileInfo],
    channel_count: usize,
    tile_size: u64,
    fill_value: u8,
) -> Result<(), KfbError> {
    let ts = tile_size as usize;
    let chunks = group_tiles_by_spatial_chunk(tiles, tile_size);

    chunks
        .into_par_iter()
        .try_for_each(|((cy, cx), chunk_tiles)| {
            let mut chunk = vec![fill_value; channel_count * ts * ts];
            for tile in chunk_tiles {
                let jpeg = reader.read_tile_bytes(tile)?;
                let (luma, raw_w, raw_h) = decode_jpeg_luma(jpeg)?;
                copy_transposed_luma_plane(
                    &mut chunk,
                    &luma,
                    tile.channel_index(),
                    raw_w,
                    raw_h,
                    ts,
                    ts,
                );
            }
            array
                .store_chunk_elements::<u8>(&[0, cy, cx], &chunk)
                .map_err(zarr_err)
        })
}

fn write_brightfield_level(
    array: &Array<zarrs::filesystem::FilesystemStore>,
    reader: &KfbReader,
    tiles: &[TileInfo],
    tile_size: u64,
    fill_value: u8,
) -> Result<(), KfbError> {
    tiles.par_iter().try_for_each(|tile| {
        let jpeg = reader.read_tile_bytes(tile)?;
        let (pixels, tile_w, tile_h) = decode_jpeg(jpeg)?;
        let cy = tile.pos_y() as u64 / tile_size;
        let cx = tile.pos_x() as u64 / tile_size;
        let ts = tile_size as usize;
        let chw = hwc_to_chw_padded(&pixels, tile_w, tile_h, ts, ts, fill_value);
        array
            .store_chunk_elements::<u8>(&[0, cy, cx], &chw)
            .map_err(zarr_err)
    })
}

pub(crate) fn write_ome_zarr(
    output: &Path,
    reader: &KfbReader,
    header: &KfbHeader,
    tiles_by_level: &[Vec<TileInfo>],
    associated_images: &[DecodedAssociatedImage],
) -> Result<(), KfbError> {
    let tile_size = infer_chunk_tile_size(reader, header, tiles_by_level)?;
    let level_dimensions: Vec<_> = tiles_by_level
        .iter()
        .enumerate()
        .map(|(i, tiles)| tile_level_dimensions(header, i, tiles))
        .collect();

    write_ome_zarr_with_level_writer(
        output,
        header,
        tile_size,
        &level_dimensions,
        associated_images,
        |i, array, fill_value| {
            let tiles = &tiles_by_level[i];
            if header.is_fluorescence() {
                write_fluorescence_level(
                    array,
                    reader,
                    tiles,
                    header.channel_count(),
                    tile_size,
                    fill_value,
                )
            } else {
                write_brightfield_level(array, reader, tiles, tile_size, fill_value)
            }
        },
    )
}

fn infer_chunk_tile_size(
    reader: &KfbReader,
    header: &KfbHeader,
    tiles_by_level: &[Vec<TileInfo>],
) -> Result<u64, KfbError> {
    let Some(tile) = tiles_by_level.iter().flatten().next() else {
        return Ok(header.tile_size() as u64);
    };
    let jpeg = reader.read_tile_bytes(tile)?;
    let (width, height) = if header.is_fluorescence() {
        let (_, width, height) = decode_jpeg_luma(jpeg)?;
        (width, height)
    } else {
        let (_, width, height) = decode_jpeg(jpeg)?;
        (width, height)
    };
    Ok(width.max(height) as u64)
}

fn write_ome_zarr_with_level_writer<F>(
    output: &Path,
    header: &KfbHeader,
    tile_size: u64,
    level_dimensions: &[(u64, u64)],
    associated_images: &[DecodedAssociatedImage],
    mut write_level: F,
) -> Result<(), KfbError>
where
    F: FnMut(usize, &Array<zarrs::filesystem::FilesystemStore>, u8) -> Result<(), KfbError>,
{
    let store = Arc::new(zarrs::filesystem::FilesystemStore::new(output).map_err(zarr_err)?);

    let mpp = header.mpp();
    let channel_count = header.channel_count();
    let num_levels = level_dimensions.len();
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

    for (i, (level_w, level_h)) in level_dimensions.iter().copied().enumerate() {
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

        write_level(i, &array, fill_value)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TileInfo, TileInfoFields};

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
    fn brightfield_hwc_to_chw_clips_decoded_padding() {
        let pixels = vec![7u8; 5 * 5 * 3];

        let chunk = hwc_to_chw_padded(&pixels, 5, 5, 4, 4, 0);

        assert_eq!(chunk.len(), 3 * 4 * 4);
        assert!(chunk.iter().all(|&b| b == 7));
    }

    #[test]
    fn fluorescence_groups_channels_by_spatial_chunk() {
        let tiles = [
            TileInfo::from_fields(TileInfoFields {
                pos_x: 0,
                pos_y: 0,
                width: 4,
                height: 4,
                channel_index: 0,
                zoom_level: 0,
                data_offset: 10,
                data_length: 4,
            }),
            TileInfo::from_fields(TileInfoFields {
                pos_x: 0,
                pos_y: 0,
                width: 4,
                height: 4,
                channel_index: 1,
                zoom_level: 0,
                data_offset: 20,
                data_length: 4,
            }),
            TileInfo::from_fields(TileInfoFields {
                pos_x: 4,
                pos_y: 0,
                width: 4,
                height: 4,
                channel_index: 0,
                zoom_level: 0,
                data_offset: 30,
                data_length: 4,
            }),
        ];

        let grouped = group_tiles_by_spatial_chunk(&tiles, 4);

        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped[&(0, 0)]
                .iter()
                .map(|tile| tile.channel_index())
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(grouped[&(0, 1)][0].data_offset, 30);
    }
}
