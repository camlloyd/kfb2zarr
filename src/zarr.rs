use std::fs;
use std::path::Path;
use std::sync::Arc;

use ome_zarr_metadata::v0_4::{
    Axis, AxisType, AxisUnit, AxisUnitSpace, Channel, Color, CoordinateTransform,
    CoordinateTransformScale, MultiscaleImage, MultiscaleImageDataset, OmeNgffGroupAttributes,
    Omero, Window,
};
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

fn base_level_dimensions(header: &KfbHeader, tiles: &[TileInfo]) -> (u64, u64) {
    let header_bounds = (header.base_width() as u64, header.base_height() as u64);
    tiles.iter().fold(header_bounds, |(w, h), tile| {
        let max_x = (tile.pos_x() + tile.width()).max(0) as u64;
        let max_y = (tile.pos_y() + tile.height()).max(0) as u64;
        (w.max(max_x), h.max(max_y))
    })
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
    let plane = dst_h * dst_w;

    let (r_plane, rest) = chw.split_at_mut(plane);
    let (g_plane, b_plane) = rest.split_at_mut(plane);

    for y in 0..copy_h {
        let src_row = &hwc[y * src_w * 3..][..copy_w * 3];
        let r_row = &mut r_plane[y * dst_w..][..dst_w];
        let g_row = &mut g_plane[y * dst_w..][..dst_w];
        let b_row = &mut b_plane[y * dst_w..][..dst_w];

        for (x, px) in src_row.chunks_exact(3).enumerate() {
            r_row[x] = px[0];
            g_row[x] = px[1];
            b_row[x] = px[2];
        }
    }
    chw
}

fn copy_luma_plane(
    chunk: &mut [u8],
    luma: &[u8],
    channel_index: usize,
    raw_w: usize,
    raw_h: usize,
    dst_w: usize,
    dst_h: usize,
) {
    let plane_start = channel_index * dst_h * dst_w;
    let copy_w = raw_w.min(dst_w);
    let copy_h = raw_h.min(dst_h);
    for raw_y in 0..copy_h {
        let src = &luma[raw_y * raw_w..][..copy_w];
        let dst = &mut chunk[plane_start + raw_y * dst_w..][..dst_w];
        dst[..copy_w].copy_from_slice(src);
    }
}

fn associated_image_name(kind: AssociatedImageKind) -> &'static str {
    match kind {
        AssociatedImageKind::Label => "label",
        AssociatedImageKind::Thumbnail => "thumbnail",
    }
}

fn omero_channel(color: Color, label: &str) -> Channel {
    Channel {
        color,
        window: Window {
            min: 0.,
            max: 255.,
            start: 0.,
            end: 255.,
        },
        other: serde_json::Map::from_iter([
            ("active".into(), json!(true)),
            ("coefficient".into(), json!(1)),
            ("family".into(), json!("linear")),
            ("inverted".into(), json!(false)),
            ("label".into(), json!(label)),
        ]),
    }
}

fn rgb_omero_channels() -> Vec<Channel> {
    vec![
        omero_channel(Color { r: 255, g: 0, b: 0 }, "R"),
        omero_channel(Color { r: 0, g: 255, b: 0 }, "G"),
        omero_channel(Color { r: 0, g: 0, b: 255 }, "B"),
    ]
}

fn write_associated_image(
    store: Arc<zarrs::filesystem::FilesystemStore>,
    image: &DecodedAssociatedImage,
    compressor: MetadataV2,
) -> Result<(), KfbError> {
    let name = associated_image_name(image.kind);
    let group_path = format!("/associated/{name}");
    let array_path = format!("{group_path}/0");
    let ome_attrs = OmeNgffGroupAttributes {
        multiscales: Some(vec![MultiscaleImage {
            version: Default::default(),
            name: Some(name.to_string()),
            axes: vec![
                Axis {
                    name: "c".into(),
                    r#type: Some(AxisType::Channel),
                    unit: None,
                },
                Axis {
                    name: "y".into(),
                    r#type: Some(AxisType::Space),
                    unit: None,
                },
                Axis {
                    name: "x".into(),
                    r#type: Some(AxisType::Space),
                    unit: None,
                },
            ],
            datasets: vec![MultiscaleImageDataset {
                path: "0".into(),
                coordinate_transformations: vec![CoordinateTransform::Scale(
                    CoordinateTransformScale::List {
                        scale: vec![1.0, 1.0, 1.0],
                    },
                )],
            }],
            coordinate_transformations: None,
            r#type: None,
            metadata: None,
        }]),
        omero: Some(Omero {
            channels: rgb_omero_channels(),
            other: serde_json::Map::from_iter([
                ("id".into(), json!(1)),
                ("name".into(), json!(name)),
                ("version".into(), json!("0.4")),
                (
                    "rdefs".into(),
                    json!({"defaultT": 0, "defaultZ": 0, "model": "color"}),
                ),
            ]),
        }),
        ..Default::default()
    };
    let serde_json::Value::Object(attrs) = serde_json::to_value(ome_attrs).map_err(zarr_err)?
    else {
        unreachable!()
    };

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
                copy_luma_plane(
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
    let base_dimensions =
        base_level_dimensions(header, tiles_by_level.first().map_or(&[], Vec::as_slice));
    let level_dimensions: Vec<_> = tiles_by_level
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let scale_factor = 1u64 << i;
            (
                base_dimensions.0.div_ceil(scale_factor),
                base_dimensions.1.div_ceil(scale_factor),
            )
        })
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

    let axes = vec![
        Axis {
            name: "c".into(),
            r#type: Some(AxisType::Channel),
            unit: None,
        },
        Axis {
            name: "y".into(),
            r#type: Some(AxisType::Space),
            unit: Some(AxisUnit::Space(AxisUnitSpace::Micrometer)),
        },
        Axis {
            name: "x".into(),
            r#type: Some(AxisType::Space),
            unit: Some(AxisUnit::Space(AxisUnitSpace::Micrometer)),
        },
    ];
    let datasets: Vec<MultiscaleImageDataset> = (0..num_levels)
        .map(|i| {
            let scale_factor = (1u64 << i) as f64;
            MultiscaleImageDataset {
                path: i.to_string(),
                coordinate_transformations: vec![CoordinateTransform::Scale(
                    CoordinateTransformScale::List {
                        scale: vec![
                            1.0,
                            (mpp * scale_factor) as f32,
                            (mpp * scale_factor) as f32,
                        ],
                    },
                )],
            }
        })
        .collect();
    let channels: Vec<Channel> = if !header.channels().is_empty() {
        header
            .channels()
            .iter()
            .map(|ch| {
                omero_channel(
                    Color {
                        r: ch.color_rgb[0],
                        g: ch.color_rgb[1],
                        b: ch.color_rgb[2],
                    },
                    &ch.name,
                )
            })
            .collect()
    } else if header.is_fluorescence() {
        let fluorescence_colors = [
            Color { r: 0, g: 0, b: 255 },
            Color { r: 0, g: 255, b: 0 },
            Color { r: 255, g: 0, b: 0 },
            Color {
                r: 255,
                g: 255,
                b: 0,
            },
            Color {
                r: 255,
                g: 0,
                b: 255,
            },
            Color {
                r: 0,
                g: 255,
                b: 255,
            },
        ];
        (0..channel_count)
            .map(|i| {
                omero_channel(
                    fluorescence_colors[i % fluorescence_colors.len()],
                    &format!("Channel {}", i + 1),
                )
            })
            .collect()
    } else {
        rgb_omero_channels()
    };

    let associated_attrs: Vec<_> = associated_images
        .iter()
        .map(|image| {
            let name = associated_image_name(image.kind);
            json!({"kind": name, "path": format!("associated/{name}")})
        })
        .collect();

    let ome_attrs = OmeNgffGroupAttributes {
        multiscales: Some(vec![MultiscaleImage {
            version: Default::default(),
            name: Some(
                output
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
            axes,
            datasets,
            coordinate_transformations: None,
            r#type: None,
            metadata: None,
        }]),
        omero: Some(Omero {
            channels,
            other: serde_json::Map::from_iter([
                ("id".into(), json!(1)),
                (
                    "name".into(),
                    json!(
                        output
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref()
                    ),
                ),
                ("version".into(), json!("0.4")),
                (
                    "rdefs".into(),
                    json!({"defaultT": 0, "defaultZ": 0, "model": "color"}),
                ),
            ]),
        }),
        ..Default::default()
    };
    let serde_json::Value::Object(mut attrs) = serde_json::to_value(ome_attrs).map_err(zarr_err)?
    else {
        unreachable!()
    };
    if !associated_attrs.is_empty() {
        attrs.insert("associated_images".into(), json!(associated_attrs));
    }

    let compressor: MetadataV2 = serde_json::from_value(json!({
        "id": "blosc",
        "cname": "zstd",
        "clevel": 1,
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
    fn fluorescence_luma_plane_copies_directly() {
        let luma = vec![1u8, 2, 3, 4, 5, 6];
        let mut chunk = vec![0u8; 4 * 4];

        copy_luma_plane(&mut chunk, &luma, 0, 3, 2, 4, 4);

        assert_eq!(&chunk[0..4], &[1, 2, 3, 0]);
        assert_eq!(&chunk[4..8], &[4, 5, 6, 0]);
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
