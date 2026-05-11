mod common;

use std::io::Write;
use std::path::{Path, PathBuf};

use byteorder::{BigEndian, LittleEndian, WriteBytesExt};
use serde_json::{Value, json};
use tempfile::{NamedTempFile, TempDir};

#[test]
fn convert_to_zarr_is_public() {
    let _: fn(&std::path::Path, &std::path::Path) -> Result<(), kfb2zarr::KfbError> =
        kfb2zarr::convert_to_zarr;
}

fn convert(input: &Path) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let output = dir.path().join("test.zarr");
    kfb2zarr::convert_to_zarr(input, &output).unwrap();
    (dir, output)
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn synth_brightfield_kfb(section: common::HeaderSection) -> NamedTempFile {
    synth_brightfield_kfb_with_label(section, None)
}

fn synth_brightfield_kfb_with_label(
    section: common::HeaderSection,
    label: Option<(i32, i32, &[u8])>,
) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    let mut buf = common::make_header_section(section);

    if let Some((w, h, jpeg)) = label {
        // Patch label pointer at 0x38 to point at the upcoming label block.
        let label_offset = buf.len() as u32;
        buf[0x38..0x3c].copy_from_slice(&label_offset.to_le_bytes());

        let label_section =
            common::make_associated_image_section(common::LABEL_START, w, h, jpeg.len() as i32);
        buf.extend_from_slice(&label_section);
        buf.extend_from_slice(jpeg);
    }

    let jpeg_offset = buf.len() as i64;
    buf.extend_from_slice(common::RED_1X1_JPEG);

    let tile_info_pos = buf.len() as i64;
    let offset_from_tile_info = jpeg_offset - tile_info_pos;
    let tile_info = common::make_tile_info_section(
        0,
        1,
        1,
        20.0,
        common::RED_1X1_JPEG.len() as i32,
        offset_from_tile_info,
    );
    buf.extend_from_slice(&tile_info);

    f.write_all(&buf).unwrap();
    f.flush().unwrap();
    f
}

fn synth_brightfield_two_level_kfb() -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    let mut buf = common::make_header_section(common::HeaderSection {
        tile_count: 2,
        base_width: 2,
        base_height: 2,
        scan_scale: 20,
        image_cap_res: 0.25,
        tile_size: 1,
        ..Default::default()
    });

    let jpeg_offset_1 = buf.len() as i64;
    buf.extend_from_slice(common::RED_1X1_JPEG);
    let jpeg_offset_2 = buf.len() as i64;
    buf.extend_from_slice(common::RED_1X1_JPEG);

    let first_tile_info_pos = buf.len() as i64;
    let tile_info_1 = common::make_tile_info_section(
        0,
        1,
        1,
        20.0,
        common::RED_1X1_JPEG.len() as i32,
        jpeg_offset_1 - first_tile_info_pos,
    );
    buf.extend_from_slice(&tile_info_1);
    let tile_info_2 = common::make_tile_info_section(
        0,
        1,
        1,
        10.0,
        common::RED_1X1_JPEG.len() as i32,
        jpeg_offset_2 - first_tile_info_pos,
    );
    buf.extend_from_slice(&tile_info_2);

    f.write_all(&buf).unwrap();
    f.flush().unwrap();
    f
}

type KfbfChannelMetadata<'a> = (&'a [&'a str], &'a [[u8; 3]], &'a [f64]);

/// Synthesize a KFBF (fluorescence) file with `channel_count` channels for one tile,
/// optionally with channel-metadata names/colors/exposures.
fn synth_kfbf(
    channel_count: usize,
    channel_metadata: Option<KfbfChannelMetadata>,
) -> NamedTempFile {
    let header_region_size = match channel_metadata {
        Some((names, colors, exposures)) => {
            let meta = common::make_kfbf_channel_metadata_block(names, colors, exposures);
            0xa8 + meta.len()
        }
        None => 0xb4,
    };
    let mut data = vec![0u8; header_region_size];
    data[0..4].copy_from_slice(&common::HEADER_START);
    data[4..8].copy_from_slice(b"KFBF");

    {
        let mut cur = std::io::Cursor::new(&mut data[0x10..]);
        cur.write_i32::<LittleEndian>(1).unwrap();
        cur.write_i32::<LittleEndian>(1).unwrap(); // height (KFBF reads height first)
        cur.write_i32::<LittleEndian>(1).unwrap(); // width
        cur.write_i32::<LittleEndian>(40).unwrap(); // scan_scale
        cur.write_all(b"JPEG").unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x4c..]);
        cur.write_f32::<LittleEndian>(0.25).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x58..]);
        cur.write_i32::<LittleEndian>(1).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0xb0..]);
        cur.write_u32::<BigEndian>(channel_count as u32).unwrap();
    }

    // Copy channel-metadata block over the slot starting at 0xa8.
    // (This intentionally overwrites the 0xb0 channel-count slot — the parser
    // re-derives channel_count from TLV block sizes when metadata is present.)
    if let Some((names, colors, exposures)) = channel_metadata {
        let meta = common::make_kfbf_channel_metadata_block(names, colors, exposures);
        data[0xa8..0xa8 + meta.len()].copy_from_slice(&meta);
    }

    let tile_info_pos = data.len();
    {
        let mut cur = std::io::Cursor::new(&mut data[0x44..]);
        cur.write_u64::<LittleEndian>(tile_info_pos as u64).unwrap();
    }
    let offset_table_pos = tile_info_pos + 64;
    let length_table_pos = offset_table_pos + channel_count * 8;
    let jpeg_payloads_pos = length_table_pos + channel_count * 8;

    let mut tile = Vec::new();
    tile.extend_from_slice(&common::TILE_INFO_START);
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap(); // x
    tile.write_i32::<LittleEndian>(1).unwrap(); // height
    tile.write_i32::<LittleEndian>(1).unwrap(); // width
    tile.write_f32::<LittleEndian>(40.0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(channel_count as i32)
        .unwrap(); // table length
    tile.write_u64::<LittleEndian>(offset_table_pos as u64)
        .unwrap();
    tile.write_u64::<LittleEndian>(length_table_pos as u64)
        .unwrap();
    tile.extend_from_slice(&[0u8; 8]);
    tile.extend_from_slice(&common::TILE_INFO_END);
    assert_eq!(tile.len(), 64);
    data.extend_from_slice(&tile);

    for i in 0..channel_count {
        data.write_u64::<LittleEndian>(
            jpeg_payloads_pos as u64 + (i as u64) * (common::RED_1X1_JPEG.len() as u64),
        )
        .unwrap();
    }
    for _ in 0..channel_count {
        data.write_u64::<LittleEndian>(common::RED_1X1_JPEG.len() as u64)
            .unwrap();
    }
    for _ in 0..channel_count {
        data.extend_from_slice(common::RED_1X1_JPEG);
    }

    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&data).unwrap();
    f.flush().unwrap();
    f
}

// =========================================================================
// brightfield, single-tile fixture
// =========================================================================

fn single_tile() -> (NamedTempFile, TempDir, PathBuf) {
    let f = synth_brightfield_kfb(common::HeaderSection {
        tile_count: 1,
        base_width: 1,
        base_height: 1,
        scan_scale: 20,
        image_cap_res: 0.25,
        tile_size: 1,
        ..Default::default()
    });
    let (dir, out) = convert(f.path());
    (f, dir, out)
}

#[test]
fn array_fill_value_is_white() {
    let (_f, _dir, out) = single_tile();
    assert_eq!(read_json(&out.join("0/.zarray"))["fill_value"], 255);
}

#[test]
fn array_shape_is_cyx() {
    let (_f, _dir, out) = single_tile();
    let z = read_json(&out.join("0/.zarray"));
    assert_eq!(z["shape"], json!([3, 1, 1]));
    assert_eq!(z["chunks"], json!([3, 1, 1]));
}

#[test]
fn chunks_use_ngff_0_4_slash_separator() {
    let (_f, _dir, out) = single_tile();
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
    let (_f, _dir, out) = single_tile();
    assert_eq!(read_json(&out.join("0/.zarray"))["dtype"], "|u1");
}

#[test]
fn array_uses_blosc_lz4_compressor() {
    let (_f, _dir, out) = single_tile();
    let z = read_json(&out.join("0/.zarray"));
    assert_eq!(z["compressor"]["id"], "blosc");
    assert_eq!(z["compressor"]["cname"], "lz4");
    assert_eq!(z["compressor"]["clevel"], 5);
    assert_eq!(z["compressor"]["shuffle"], 1);
    assert_eq!(z["compressor"]["blocksize"], 0);
}

#[test]
fn array_zattrs_record_cyx_dimensions() {
    let (_f, _dir, out) = single_tile();
    let attrs = read_json(&out.join("0/.zattrs"));
    assert_eq!(attrs["_ARRAY_DIMENSIONS"], json!(["c", "y", "x"]));
}

#[test]
fn multiscales_version_is_0_4() {
    let (_f, _dir, out) = single_tile();
    assert!(out.join(".zgroup").exists(), ".zgroup not found");
    assert_eq!(
        read_json(&out.join(".zattrs"))["multiscales"][0]["version"],
        "0.4"
    );
}

#[test]
fn axes_are_cyx() {
    let (_f, _dir, out) = single_tile();
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
fn ome_xml_records_creator_and_metadata_only() {
    let (_f, _dir, out) = single_tile();
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    let expected = format!(r#"Creator="kfb2zarr {}""#, env!("CARGO_PKG_VERSION"));
    assert!(xml.contains(&expected), "missing {expected:?} in {xml}");
    assert!(xml.contains("<MetadataOnly/>"));
}

#[test]
fn ome_xml_is_single_line() {
    let (_f, _dir, out) = single_tile();
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    assert!(!xml.contains('\n'));
    assert!(xml.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?><OME "#));
}

#[test]
fn brightfield_omero_defaults_to_three_rgb_channels() {
    let (_f, _dir, out) = single_tile();
    let channels = read_json(&out.join(".zattrs"))["omero"]["channels"].clone();
    assert_eq!(channels.as_array().unwrap().len(), 3);
    assert_eq!(channels[0]["color"], "FF0000");
    assert_eq!(channels[1]["color"], "00FF00");
    assert_eq!(channels[2]["color"], "0000FF");
}

#[test]
fn ome_xml_records_physical_size_and_magnification() {
    let (_f, _dir, out) = single_tile();
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    assert!(xml.contains(r#"NominalMagnification="20""#));
    assert!(xml.contains(r#"PhysicalSizeX="0.25""#));
    assert!(xml.contains(r#"PhysicalSizeY="0.25""#));
    assert!(xml.contains(r#"PhysicalSizeXUnit="µm""#));
    assert!(xml.contains(r#"PhysicalSizeYUnit="µm""#));
    assert!(xml.contains(r#"SizeX="1""#));
    assert!(xml.contains(r#"SizeY="1""#));
}

// =========================================================================
// brightfield, multi-level fixture
// =========================================================================

#[test]
fn each_pyramid_level_halves_resolution() {
    let f = synth_brightfield_two_level_kfb();
    let (_dir, out) = convert(f.path());
    assert_eq!(read_json(&out.join("0/.zarray"))["shape"], json!([3, 2, 2]));
    assert_eq!(read_json(&out.join("1/.zarray"))["shape"], json!([3, 1, 1]));
}

#[test]
fn datasets_match_levels_and_scale_vectors() {
    let f = synth_brightfield_two_level_kfb();
    let (_dir, out) = convert(f.path());
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

// =========================================================================
// brightfield with associated label image
// =========================================================================

#[test]
fn slide_label_is_written_as_associated_image() {
    let f = synth_brightfield_kfb_with_label(
        common::HeaderSection {
            tile_count: 1,
            base_width: 1,
            base_height: 1,
            scan_scale: 20,
            image_cap_res: 0.25,
            tile_size: 1,
            ..Default::default()
        },
        Some((1, 1, common::RED_1X1_JPEG)),
    );
    let (_dir, out) = convert(f.path());

    let root_attrs = read_json(&out.join(".zattrs"));
    assert_eq!(
        root_attrs["associated_images"],
        json!([{"kind": "label", "path": "associated/label"}])
    );
    assert_eq!(
        read_json(&out.join("associated/.zattrs"))["images"],
        json!(["label"])
    );
    assert_eq!(
        read_json(&out.join("associated/label/.zattrs"))["multiscales"][0]["datasets"][0]["path"],
        "0"
    );
    assert!(
        out.join("associated/label/0/0/0/0").exists(),
        "associated label chunk should be written"
    );
}

// =========================================================================
// brightfield with header-vs-tile-bounds mismatch
// =========================================================================

#[test]
fn shape_uses_tile_bounds_when_header_is_short() {
    // Header claims 3×3; tile is placed at y=4 size 1×1 → y bounds (5) exceed header.
    // (pos_x can't be patched per-tile from the test fixture: the reader assigns
    // pos_x = rank × 256 from section ordering, so we exercise the y-axis case.)
    let mut f = NamedTempFile::new().unwrap();
    let mut buf = common::make_header_section(common::HeaderSection {
        tile_count: 1,
        base_width: 3,
        base_height: 3,
        scan_scale: 20,
        image_cap_res: 0.25,
        tile_size: 1,
        ..Default::default()
    });
    let jpeg_offset = buf.len() as i64;
    buf.extend_from_slice(common::RED_1X1_JPEG);
    let tile_info_pos = buf.len() as i64;
    let tile_info = common::make_tile_info_section(
        4, // y_native — exceeds header height 3
        1,
        1,
        20.0,
        common::RED_1X1_JPEG.len() as i32,
        jpeg_offset - tile_info_pos,
    );
    buf.extend_from_slice(&tile_info);
    f.write_all(&buf).unwrap();
    f.flush().unwrap();

    let (_dir, out) = convert(f.path());

    assert_eq!(read_json(&out.join("0/.zarray"))["shape"], json!([3, 5, 3]));
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    assert!(xml.contains(r#"SizeX="3""#));
    assert!(xml.contains(r#"SizeY="5""#));
}

// =========================================================================
// brightfield with scan_time → AcquisitionDate
// =========================================================================

#[test]
fn ome_xml_records_acquisition_date_as_utc() {
    let f = synth_brightfield_kfb(common::HeaderSection {
        tile_count: 1,
        base_width: 1,
        base_height: 1,
        scan_scale: 20,
        scan_time: 1_773_884_060,
        image_cap_res: 0.25,
        tile_size: 1,
        ..Default::default()
    });
    let (_dir, out) = convert(f.path());
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    assert!(xml.contains("<AcquisitionDate>2026-03-19T01:34:20Z</AcquisitionDate>"));
}

// =========================================================================
// fluorescence (KFBF)
// =========================================================================

#[test]
fn fluorescence_zarr_uses_source_channel_count() {
    let f = synth_kfbf(6, None);
    let (_dir, out) = convert(f.path());
    let z = read_json(&out.join("0/.zarray"));
    assert_eq!(z["shape"], json!([6, 1, 1]));
    assert_eq!(z["chunks"], json!([6, 1, 1]));
    assert_eq!(z["fill_value"], 0);
    let channels = read_json(&out.join(".zattrs"))["omero"]["channels"].clone();
    assert_eq!(channels.as_array().unwrap().len(), 6);
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    assert!(xml.contains(r#"SizeC="6""#));
    assert!(xml.contains(r#"Name="Channel 6""#));
}

fn fluorescence_with_two_named_channels() -> (NamedTempFile, TempDir, PathBuf) {
    let names = ["DAPI", "FITC"];
    let colors = [[0u8, 0, 255], [0, 255, 0]];
    let exposures = [10.0_f64, 30.0];
    let f = synth_kfbf(2, Some((&names, &colors, &exposures)));
    let (dir, out) = convert(f.path());
    (f, dir, out)
}

#[test]
fn fluorescence_omero_uses_channel_names_and_colors() {
    let (_f, _dir, out) = fluorescence_with_two_named_channels();
    let channels = read_json(&out.join(".zattrs"))["omero"]["channels"].clone();
    assert_eq!(channels[0]["label"], "DAPI");
    assert_eq!(channels[0]["color"], "0000FF");
    assert_eq!(channels[1]["label"], "FITC");
    assert_eq!(channels[1]["color"], "00FF00");
}

#[test]
fn fluorescence_ome_xml_uses_channel_names() {
    let (_f, _dir, out) = fluorescence_with_two_named_channels();
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    assert!(xml.contains(r#"Name="DAPI""#));
    assert!(xml.contains(r#"Name="FITC""#));
    assert!(!xml.contains(r#"Name="Channel 1""#));
}

#[test]
fn fluorescence_ome_xml_uses_channel_colors() {
    let (_f, _dir, out) = fluorescence_with_two_named_channels();
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    assert!(xml.contains(r#"Name="DAPI" Color="65535""#));
    assert!(xml.contains(r#"Name="FITC" Color="16711935""#));
}

#[test]
fn fluorescence_ome_xml_has_plane_exposure_times() {
    let (_f, _dir, out) = fluorescence_with_two_named_channels();
    let xml = std::fs::read_to_string(out.join("OME/METADATA.ome.xml")).unwrap();
    assert!(
        xml.contains(r#"ExposureTime="10" ExposureTimeUnit="ms""#)
            || xml.contains(r#"ExposureTime="10.0" ExposureTimeUnit="ms""#),
        "expected DAPI exposure"
    );
    assert!(
        xml.contains(r#"ExposureTime="30" ExposureTimeUnit="ms""#)
            || xml.contains(r#"ExposureTime="30.0" ExposureTimeUnit="ms""#),
        "expected FITC exposure"
    );
    assert!(xml.contains(r#"TheC="0""#));
    assert!(xml.contains(r#"TheC="1""#));
}
