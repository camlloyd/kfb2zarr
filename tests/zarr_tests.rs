mod common;

use std::io::Write;
use tempfile::{NamedTempFile, TempDir};

#[test]
fn convert_to_zarr_is_public() {
    let _: fn(&std::path::Path, &std::path::Path) -> Result<(), kfb2zarr::KfbError> =
        kfb2zarr::convert_to_zarr;
}

#[test]
fn brightfield_kfb_round_trips_through_convert_to_zarr() {
    let mut input = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        tile_count: 1,
        base_width: 1,
        base_height: 1,
        scan_scale: 20,
        image_cap_res: 0.25,
        tile_size: 1,
        ..Default::default()
    });
    input.write_all(&header).unwrap();
    input.write_all(common::RED_1X1_JPEG).unwrap();

    let section_start = (header.len() + common::RED_1X1_JPEG.len()) as i64;
    let offset_from_file = (header.len() as i64) - section_start;
    let tile_info = common::make_tile_info_section(
        0,
        1,
        1,
        20.0,
        common::RED_1X1_JPEG.len() as i32,
        offset_from_file,
    );
    input.write_all(&tile_info).unwrap();
    input.flush().unwrap();

    let out_dir = TempDir::new().unwrap();
    let output = out_dir.path().join("slide.ome.zarr");
    kfb2zarr::convert_to_zarr(input.path(), &output).unwrap();

    assert!(output.join(".zgroup").exists(), ".zgroup missing");
    assert!(
        output.join("OME/METADATA.ome.xml").exists(),
        "OME-XML sidecar missing"
    );
    let zarray: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output.join("0/.zarray")).unwrap()).unwrap();
    assert_eq!(zarray["shape"], serde_json::json!([3, 1, 1]));
    assert!(
        output.join("0/0/0/0").exists(),
        "expected single chunk at 0/0/0/0"
    );
}
