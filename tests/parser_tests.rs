mod common;

use byteorder::{BigEndian, LittleEndian, WriteBytesExt};
use kfb2zarr::KfbError;
use kfb2zarr::KfbReader;
use kfb2zarr::types::AssociatedImageKind;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn reader_parses_kfb_header_dimensions_and_metadata() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        tile_count: 100,
        base_width: 10000,
        base_height: 8000,
        scan_scale: 40,
        spend_time: 120,
        scan_time: 1700000000,
        image_cap_res: 0.242,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.header().tile_count(), 100);
    assert_eq!(reader.header().base_width(), 10000);
    assert_eq!(reader.header().base_height(), 8000);
    assert_eq!(reader.header().scan_scale(), 40);
    assert!((reader.header().mpp() - 0.242).abs() < 1e-5);
    assert_eq!(reader.header().tile_size(), 256);
    assert_eq!(reader.header().scan_time(), 1700000000);
    assert!(reader.header().zoom_levels() > 0);
}

#[test]
fn reader_parses_tile_position_dimensions_and_zoom_level() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        spend_time: 60,
        scan_time: 1700000000,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    let fake_jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xD9];
    f.write_all(&fake_jpeg).unwrap();
    let section_start = (header.len() + fake_jpeg.len()) as i64;
    let offset_from_file = (header.len() as i64) - section_start;
    // y_native=512, tile_h=256, tile_w=256, magnification=20.0 (full res)
    let tile_info = common::make_tile_info_section(
        512,
        256,
        256,
        20.0,
        fake_jpeg.len() as i32,
        offset_from_file,
    );
    f.write_all(&tile_info).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.tiles().len(), 1);
    let tile = &reader.tiles()[0];
    assert_eq!(tile.pos_x(), 0); // rank 0 within (y=512, level=0) group → x=0
    assert_eq!(tile.pos_y(), 512); // y_native
    assert_eq!(tile.width(), 256);
    assert_eq!(tile.height(), 256);
    assert_eq!(tile.zoom_level(), 0);
}

#[test]
fn reader_partial_tile_dimensions_are_not_swapped() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        base_width: 37764,
        base_height: 25650,
        image_cap_res: 0.247488,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    let fake_jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xD9];
    f.write_all(&fake_jpeg).unwrap();
    let section_start = (header.len() + fake_jpeg.len()) as i64;
    let offset_from_file = (header.len() as i64) - section_start;
    let tile_info = common::make_tile_info_section(
        25600,
        132,
        55,
        20.0,
        fake_jpeg.len() as i32,
        offset_from_file,
    );
    f.write_all(&tile_info).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    let tile = &reader.tiles()[0];
    assert_eq!(tile.width(), 132);
    assert_eq!(tile.height(), 55);
    assert_eq!(tile.pos_y() + tile.height(), 25655);
}

#[test]
fn reader_parses_kfb_label_associated_image() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        tile_count: 0,
        spend_time: 60,
        scan_time: 1700000000,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    let fake_jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xD9];
    f.write_all(&fake_jpeg).unwrap();
    let label_section = common::make_associated_image_section(
        common::LABEL_START,
        512,
        384,
        fake_jpeg.len() as i32,
    );
    f.write_all(&label_section).unwrap();
    f.write_all(&fake_jpeg).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.associated_images().len(), 1);
    let img = &reader.associated_images()[0];
    assert_eq!(img.kind(), AssociatedImageKind::Label);
    assert_eq!(img.width(), 512);
    assert_eq!(img.height(), 384);
}

#[test]
fn reader_parses_kfb_thumbnail_associated_image() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        tile_count: 0,
        spend_time: 60,
        scan_time: 1700000000,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    let fake_jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xD9];
    let thumbnail_section = common::make_associated_image_section(
        common::THUMBNAIL_START,
        300,
        120,
        fake_jpeg.len() as i32,
    );
    f.write_all(&thumbnail_section).unwrap();
    f.write_all(&fake_jpeg).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.associated_images().len(), 1);
    assert_eq!(
        reader.associated_images()[0].kind(),
        AssociatedImageKind::Thumbnail
    );
}

#[test]
fn reader_extends_associated_image_length_to_jpeg_eoi() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        tile_count: 0,
        spend_time: 60,
        scan_time: 1700000000,
        ..Default::default()
    });
    f.write_all(&header).unwrap();

    let fake_jpeg = vec![0xFFu8, 0xD8, 1, 2, 3, 4, 0xFF, 0xD9];
    let label_section = common::make_associated_image_section(common::LABEL_START, 300, 120, 4);
    f.write_all(&label_section).unwrap();
    f.write_all(&fake_jpeg).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.associated_images().len(), 1);
    assert_eq!(
        reader
            .read_associated_bytes(&reader.associated_images()[0])
            .unwrap(),
        fake_jpeg.as_slice()
    );
}

#[test]
fn reader_ignores_marker_bytes_inside_tile_jpeg() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        spend_time: 60,
        scan_time: 1700000000,
        ..Default::default()
    });
    f.write_all(&header).unwrap();

    let mut fake_jpeg = vec![0u8; 80];
    fake_jpeg[0..2].copy_from_slice(&[0xFF, 0xD8]);
    fake_jpeg[10..14].copy_from_slice(&common::THUMBNAIL_START);
    fake_jpeg[22..26].copy_from_slice(&120i32.to_le_bytes());
    fake_jpeg[26..30].copy_from_slice(&300i32.to_le_bytes());
    fake_jpeg[34..38].copy_from_slice(&52i32.to_le_bytes());
    fake_jpeg[62..64].copy_from_slice(&[0xFF, 0xD8]);
    fake_jpeg[78..80].copy_from_slice(&[0xFF, 0xD9]);
    f.write_all(&fake_jpeg).unwrap();

    let section_start = (header.len() + fake_jpeg.len()) as i64;
    let offset_from_file = (header.len() as i64) - section_start;
    let tile_info =
        common::make_tile_info_section(0, 256, 256, 20.0, fake_jpeg.len() as i32, offset_from_file);
    f.write_all(&tile_info).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.tiles().len(), 1);
    assert!(
        reader.associated_images().is_empty(),
        "marker-like bytes inside tile JPEG data must not be parsed as associated images"
    );
}

#[test]
fn reader_parses_kfbf_channel_tiles_from_indirection_tables() {
    let mut f = NamedTempFile::new().unwrap();
    let mut data = vec![0u8; 192];
    data[0..4].copy_from_slice(&common::HEADER_START);
    data[4..8].copy_from_slice(b"KFBF");
    {
        let mut cur = std::io::Cursor::new(&mut data[0x10..]);
        cur.write_i32::<LittleEndian>(1).unwrap(); // tile records
        cur.write_i32::<LittleEndian>(1024).unwrap(); // width slot for KFBF
        cur.write_i32::<LittleEndian>(512).unwrap(); // height slot for KFBF
        cur.write_i32::<LittleEndian>(40).unwrap();
        cur.write_all(b"JPEG").unwrap();
        cur.write_i32::<LittleEndian>(0).unwrap();
        cur.write_i64::<LittleEndian>(0).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x34..]);
        cur.write_i32::<LittleEndian>(275).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x44..]);
        cur.write_u64::<LittleEndian>(192).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x4c..]);
        cur.write_f32::<LittleEndian>(0.25).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x58..]);
        cur.write_i32::<LittleEndian>(512).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0xb0..]);
        cur.write_u32::<BigEndian>(6).unwrap();
    }

    let offset_table = 256u64;
    let length_table = 304u64;
    let jpeg_start = 352u64;
    let mut tile = Vec::new();
    tile.extend_from_slice(&common::TILE_INFO_START);
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap(); // x
    tile.write_i32::<LittleEndian>(512).unwrap(); // height
    tile.write_i32::<LittleEndian>(512).unwrap(); // width
    tile.write_f32::<LittleEndian>(40.0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(4).unwrap();
    tile.write_u64::<LittleEndian>(offset_table).unwrap();
    tile.write_u64::<LittleEndian>(length_table).unwrap();
    tile.extend_from_slice(&[0u8; 8]);
    tile.extend_from_slice(&common::TILE_INFO_END);
    assert_eq!(tile.len(), 64);
    data.extend_from_slice(&tile);
    data.extend_from_slice(&[0u8; 0]);
    assert_eq!(data.len(), offset_table as usize);
    for channel in 0..6u64 {
        data.write_u64::<LittleEndian>(jpeg_start + channel * 4)
            .unwrap();
    }
    assert_eq!(data.len(), length_table as usize);
    for _ in 0..6 {
        data.write_u64::<LittleEndian>(4).unwrap();
    }
    assert_eq!(data.len(), jpeg_start as usize);
    for channel in 0..6u8 {
        data.extend_from_slice(&[0xFF, 0xD8, channel, 0xD9]);
    }

    f.write_all(&data).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.header().base_width(), 1024);
    assert_eq!(reader.header().base_height(), 512);
    assert_eq!(reader.header().tile_size(), 512);
    assert_eq!(reader.header().channel_count(), 6);
    assert_eq!(reader.tiles().len(), 6);
    for (channel, tile) in reader.tiles().iter().enumerate() {
        assert_eq!(tile.channel_index(), channel);
        assert_eq!(tile.width(), 512);
        assert_eq!(tile.height(), 512);
        assert_eq!(reader.read_tile_bytes(tile).unwrap()[2], channel as u8);
    }
}

#[test]
fn reader_parses_kfbf_thumbnail_associated_image() {
    let mut f = NamedTempFile::new().unwrap();
    let mut data = vec![0u8; 192];
    data[0..4].copy_from_slice(&common::HEADER_START);
    data[4..8].copy_from_slice(b"KFBF");
    {
        let mut cur = std::io::Cursor::new(&mut data[0x10..]);
        cur.write_i32::<LittleEndian>(0).unwrap();
        cur.write_i32::<LittleEndian>(1024).unwrap();
        cur.write_i32::<LittleEndian>(512).unwrap();
        cur.write_i32::<LittleEndian>(40).unwrap();
        cur.write_all(b"JPEG").unwrap();
    }
    {
        let tile_index_offset = data.len() as u64;
        let mut cur = std::io::Cursor::new(&mut data[0x44..]);
        cur.write_u64::<LittleEndian>(tile_index_offset).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x58..]);
        cur.write_i32::<LittleEndian>(512).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0xb0..]);
        cur.write_u32::<BigEndian>(6).unwrap();
    }

    let mut section = Vec::new();
    section.extend_from_slice(&common::THUMBNAIL_START);
    section.write_i32::<LittleEndian>(1).unwrap();
    section.write_i32::<LittleEndian>(536).unwrap();
    section.write_i32::<LittleEndian>(560).unwrap();
    section.write_i32::<LittleEndian>(3).unwrap();
    section.write_i32::<LittleEndian>(4).unwrap();
    section.write_i32::<LittleEndian>(52).unwrap();
    section.extend_from_slice(&[0u8; 24]);
    assert_eq!(section.len(), 52);
    section.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xD9]);
    data.extend_from_slice(&section);

    f.write_all(&data).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.associated_images().len(), 1);
    let img = &reader.associated_images()[0];
    assert_eq!(img.kind(), AssociatedImageKind::Thumbnail);
    assert_eq!(img.width(), 560);
    assert_eq!(img.height(), 536);
    assert_eq!(
        reader.read_associated_bytes(img).unwrap(),
        &[0xFF, 0xD8, 0xFF, 0xD9]
    );
}

#[test]
fn reader_parses_kfbf_label_associated_image() {
    let mut f = NamedTempFile::new().unwrap();
    let mut data = vec![0u8; 192];
    data[0..4].copy_from_slice(&common::HEADER_START);
    data[4..8].copy_from_slice(b"KFBF");
    {
        let mut cur = std::io::Cursor::new(&mut data[0x10..]);
        cur.write_i32::<LittleEndian>(0).unwrap();
        cur.write_i32::<LittleEndian>(1024).unwrap();
        cur.write_i32::<LittleEndian>(512).unwrap();
        cur.write_i32::<LittleEndian>(40).unwrap();
        cur.write_all(b"JPEG").unwrap();
    }
    {
        let tile_index_offset = data.len() as u64;
        let mut cur = std::io::Cursor::new(&mut data[0x44..]);
        cur.write_u64::<LittleEndian>(tile_index_offset).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x58..]);
        cur.write_i32::<LittleEndian>(512).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0xb0..]);
        cur.write_u32::<BigEndian>(6).unwrap();
    }

    let mut section = Vec::new();
    section.extend_from_slice(&common::LABEL_START);
    section.write_i32::<LittleEndian>(1).unwrap();
    section.write_i32::<LittleEndian>(300).unwrap();
    section.write_i32::<LittleEndian>(400).unwrap();
    section.write_i32::<LittleEndian>(3).unwrap();
    section.write_i32::<LittleEndian>(4).unwrap();
    section.write_i32::<LittleEndian>(52).unwrap();
    section.extend_from_slice(&[0u8; 24]);
    assert_eq!(section.len(), 52);
    section.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xD9]);
    data.extend_from_slice(&section);

    f.write_all(&data).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.associated_images().len(), 1);
    let img = &reader.associated_images()[0];
    assert_eq!(img.kind(), AssociatedImageKind::Label);
    assert_eq!(img.width(), 400);
    assert_eq!(img.height(), 300);
    assert_eq!(
        reader.read_associated_bytes(img).unwrap(),
        &[0xFF, 0xD8, 0xFF, 0xD9]
    );
}

fn open_with_tile_size(base_width: i32, base_height: i32, tile_size: i32) -> KfbReader {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&common::make_header_section(common::HeaderSection {
        base_width,
        base_height,
        tile_size,
        ..Default::default()
    }))
    .unwrap();
    f.flush().unwrap();
    let reader = KfbReader::open(f.path()).unwrap();
    drop(f);
    reader
}

#[test]
fn zoom_levels_scales_with_256px_tile_size() {
    assert_eq!(
        open_with_tile_size(65536, 49152, 256)
            .header()
            .zoom_levels(),
        9
    );
}

#[test]
fn zoom_levels_scales_with_512px_tile_size() {
    assert_eq!(
        open_with_tile_size(65536, 49152, 512)
            .header()
            .zoom_levels(),
        8
    );
}

#[test]
fn zoom_levels_returns_one_when_image_fits_in_single_tile() {
    assert_eq!(open_with_tile_size(128, 64, 256).header().zoom_levels(), 1);
}

#[test]
fn zoom_levels_clamps_zero_tile_size_to_one_pixel() {
    // tile_size=0 is clamped to 1; effective depth = ceil(log2(256/1)) + 1 = 9
    assert_eq!(open_with_tile_size(256, 256, 0).header().zoom_levels(), 9);
}

#[test]
fn open_nonexistent_file_returns_io_error() {
    let err = KfbReader::open(std::path::Path::new("/nonexistent/file.kfb")).unwrap_err();
    assert!(
        matches!(err, KfbError::Io(_)),
        "expected Io error, got {err:?}"
    );
}

#[test]
fn open_wrong_magic_returns_invalid_magic() {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&[0x00u8; 96]).unwrap(); // 96 zero bytes — wrong magic
    f.flush().unwrap();

    let err = KfbReader::open(f.path()).unwrap_err();
    assert!(
        matches!(err, KfbError::InvalidMagic { .. }),
        "expected InvalidMagic, got {err:?}"
    );
}

#[test]
fn read_tile_bytes_out_of_bounds_returns_invalid_offset() {
    let mut f = NamedTempFile::new().unwrap();
    // Header pointing to a tile at offset 9999 — beyond end of file
    let header = common::make_header_section(common::HeaderSection {
        spend_time: 60,
        scan_time: 1700000000,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    // Tile info: offset_from_file points far past end of file
    let offset_from_file: i64 = 9000; // section_start + 9000 >> file size
    let tile_info = common::make_tile_info_section(0, 256, 256, 20.0, 1024, offset_from_file);
    f.write_all(&tile_info).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.tiles().len(), 1);
    let err = reader.read_tile_bytes(&reader.tiles()[0]).unwrap_err();
    assert!(
        matches!(err, KfbError::InvalidOffset { .. }),
        "expected InvalidOffset, got {err:?}"
    );
}

// Two tiles in the same (y_native, zoom_level) group must receive sequential
// x positions (0 and 256) based on their order in the file — x is not stored
// in the tile section so it is derived from section rank.
#[test]
fn tiles_in_same_row_get_sequential_x_positions() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        tile_count: 2,
        base_width: 512,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    let fake_jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xD9];
    f.write_all(&fake_jpeg).unwrap();
    f.write_all(&fake_jpeg).unwrap();

    // Both tiles: same y_native (0) and magnification (20.0) → same group.
    let data_area_start = (header.len() + fake_jpeg.len() * 2) as i64;
    let jpeg0_abs = header.len() as i64;
    let jpeg1_abs = jpeg0_abs + fake_jpeg.len() as i64;
    let section0_abs = data_area_start;
    let section1_abs =
        data_area_start + common::make_tile_info_section(0, 256, 256, 20.0, 1, 0).len() as i64;

    let tile0 = common::make_tile_info_section(
        0,
        256,
        256,
        20.0,
        fake_jpeg.len() as i32,
        jpeg0_abs - section0_abs,
    );
    let tile1 = common::make_tile_info_section(
        0,
        256,
        256,
        20.0,
        fake_jpeg.len() as i32,
        jpeg1_abs - section1_abs,
    );
    f.write_all(&tile0).unwrap();
    f.write_all(&tile1).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.tiles().len(), 2);
    assert_eq!(reader.tiles()[0].pos_x(), 0, "first tile in row → x=0");
    assert_eq!(reader.tiles()[1].pos_x(), 256, "second tile in row → x=256");
    assert_eq!(reader.tiles()[0].pos_y(), 0);
    assert_eq!(reader.tiles()[1].pos_y(), 0);
}

// Tiles at half and quarter magnification must be assigned zoom levels 1 and 2.
#[test]
fn magnification_maps_to_zoom_level() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        tile_count: 3,
        base_width: 512,
        base_height: 512,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    let fake_jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xD9];
    for _ in 0..3 {
        f.write_all(&fake_jpeg).unwrap();
    }

    let data_area_start = (header.len() + fake_jpeg.len() * 3) as i64;
    let section_len = common::make_tile_info_section(0, 256, 256, 20.0, 1, 0).len() as i64;
    for (idx, &mag) in [20.0f32, 10.0, 5.0].iter().enumerate() {
        let jpeg_abs = header.len() as i64 + idx as i64 * fake_jpeg.len() as i64;
        let sec_abs = data_area_start + idx as i64 * section_len;
        let section = common::make_tile_info_section(
            0,
            256,
            256,
            mag,
            fake_jpeg.len() as i32,
            jpeg_abs - sec_abs,
        );
        f.write_all(&section).unwrap();
    }
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    let zoom_levels: Vec<i32> = reader.tiles().iter().map(|t| t.zoom_level()).collect();
    assert_eq!(zoom_levels, vec![0, 1, 2], "20×→0, 10×→1, 5×→2");
}

// A 40× scanner (scan_scale=40) should map mag=40 → level 0, mag=20 → level 1.
#[test]
fn zoom_level_respects_scanner_max_magnification() {
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection {
        tile_count: 2,
        base_width: 512,
        base_height: 512,
        scan_scale: 40,
        image_cap_res: 0.12,
        ..Default::default()
    });
    f.write_all(&header).unwrap();
    let fake_jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xD9];
    for _ in 0..2 {
        f.write_all(&fake_jpeg).unwrap();
    }

    let data_area_start = (header.len() + fake_jpeg.len() * 2) as i64;
    let section_len = common::make_tile_info_section(0, 256, 256, 40.0, 1, 0).len() as i64;
    for (idx, &mag) in [40.0f32, 20.0].iter().enumerate() {
        let jpeg_abs = header.len() as i64 + idx as i64 * fake_jpeg.len() as i64;
        let sec_abs = data_area_start + idx as i64 * section_len;
        let section = common::make_tile_info_section(
            0,
            256,
            256,
            mag,
            fake_jpeg.len() as i32,
            jpeg_abs - sec_abs,
        );
        f.write_all(&section).unwrap();
    }
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.tiles()[0].zoom_level(), 0, "40× tile → level 0");
    assert_eq!(
        reader.tiles()[1].zoom_level(),
        1,
        "20× tile on 40× scanner → level 1"
    );
}

#[test]
fn reader_kfbf_channels_empty_when_metadata_tags_missing() {
    let mut f = NamedTempFile::new().unwrap();
    let mut data = vec![0u8; 192];
    data[0..4].copy_from_slice(&common::HEADER_START);
    data[4..8].copy_from_slice(b"KFBF");
    {
        let mut cur = std::io::Cursor::new(&mut data[0x10..]);
        cur.write_i32::<LittleEndian>(1).unwrap();
        cur.write_i32::<LittleEndian>(512).unwrap();
        cur.write_i32::<LittleEndian>(1024).unwrap();
        cur.write_i32::<LittleEndian>(40).unwrap();
        cur.write_all(b"JPEG").unwrap();
        cur.write_i32::<LittleEndian>(0).unwrap();
        cur.write_i64::<LittleEndian>(0).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x44..]);
        cur.write_u64::<LittleEndian>(192).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x58..]);
        cur.write_i32::<LittleEndian>(512).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0xb0..]);
        cur.write_u32::<BigEndian>(2).unwrap();
    }
    let offset_table = 256u64;
    let length_table = 272u64;
    let jpeg_start = 288u64;
    let mut tile = Vec::new();
    tile.extend_from_slice(&common::TILE_INFO_START);
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(512).unwrap();
    tile.write_i32::<LittleEndian>(512).unwrap();
    tile.write_f32::<LittleEndian>(40.0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(4).unwrap();
    tile.write_u64::<LittleEndian>(offset_table).unwrap();
    tile.write_u64::<LittleEndian>(length_table).unwrap();
    tile.extend_from_slice(&[0u8; 8]);
    tile.extend_from_slice(&common::TILE_INFO_END);
    data.extend_from_slice(&tile);
    assert_eq!(data.len(), offset_table as usize);
    for ch in 0..2u64 {
        data.write_u64::<LittleEndian>(jpeg_start + ch * 4).unwrap();
    }
    assert_eq!(data.len(), length_table as usize);
    for _ in 0..2 {
        data.write_u64::<LittleEndian>(4).unwrap();
    }
    assert_eq!(data.len(), jpeg_start as usize);
    data.extend_from_slice(&[0xFF, 0xD8, 0x00, 0xD9]);
    data.extend_from_slice(&[0xFF, 0xD8, 0x01, 0xD9]);
    f.write_all(&data).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert!(
        reader.header().channels().is_empty(),
        "expected no channel metadata when TLV block is absent"
    );
    assert_eq!(
        reader.header().channel_count(),
        2,
        "channel_count must be preserved even when metadata is absent"
    );
}

#[test]
fn reader_parses_kfbf_channel_names_colors_and_exposures() {
    let mut f = NamedTempFile::new().unwrap();

    let names = ["DAPI", "FITC"];
    let colors: [[u8; 3]; 2] = [[0, 0, 255], [0, 255, 0]];
    let exposures = [10.0f64, 30.0f64];
    let channel_count = 2usize;

    let meta_block = common::make_kfbf_channel_metadata_block(&names, &colors, &exposures);
    // header region must hold bytes 0..0xa8 + meta_block
    let header_region = 0xa8 + meta_block.len();
    let mut data = vec![0u8; header_region];
    data[0..4].copy_from_slice(&common::HEADER_START);
    data[4..8].copy_from_slice(b"KFBF");
    {
        let mut cur = std::io::Cursor::new(&mut data[0x10..]);
        cur.write_i32::<LittleEndian>(1).unwrap();
        cur.write_i32::<LittleEndian>(512).unwrap();
        cur.write_i32::<LittleEndian>(1024).unwrap();
        cur.write_i32::<LittleEndian>(40).unwrap();
        cur.write_all(b"JPEG").unwrap();
        cur.write_i32::<LittleEndian>(0).unwrap();
        cur.write_i64::<LittleEndian>(0).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0x58..]);
        cur.write_i32::<LittleEndian>(512).unwrap();
    }
    {
        let mut cur = std::io::Cursor::new(&mut data[0xb0..]);
        cur.write_u32::<BigEndian>(channel_count as u32).unwrap();
    }
    let tlv_base: usize = 0xa8;
    data[tlv_base..tlv_base + meta_block.len()].copy_from_slice(&meta_block);

    let tile_index_offset = header_region as u64;
    {
        let mut cur = std::io::Cursor::new(&mut data[0x44..]);
        cur.write_u64::<LittleEndian>(tile_index_offset).unwrap();
    }
    let offset_table = tile_index_offset + 64;
    let length_table = offset_table + channel_count as u64 * 8;
    let jpeg_start = length_table + channel_count as u64 * 8;

    let mut tile = Vec::new();
    tile.extend_from_slice(&common::TILE_INFO_START);
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(512).unwrap();
    tile.write_i32::<LittleEndian>(512).unwrap();
    tile.write_f32::<LittleEndian>(40.0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(0).unwrap();
    tile.write_i32::<LittleEndian>(4).unwrap();
    tile.write_u64::<LittleEndian>(offset_table).unwrap();
    tile.write_u64::<LittleEndian>(length_table).unwrap();
    tile.extend_from_slice(&[0u8; 8]);
    tile.extend_from_slice(&common::TILE_INFO_END);
    assert_eq!(tile.len(), 64);
    data.extend_from_slice(&tile);

    assert_eq!(data.len(), offset_table as usize);
    for ch in 0..channel_count as u64 {
        data.write_u64::<LittleEndian>(jpeg_start + ch * 4).unwrap();
    }
    assert_eq!(data.len(), length_table as usize);
    for _ in 0..channel_count {
        data.write_u64::<LittleEndian>(4).unwrap();
    }
    assert_eq!(data.len(), jpeg_start as usize);
    for ch in 0..channel_count as u8 {
        data.extend_from_slice(&[0xFF, 0xD8, ch, 0xD9]);
    }

    f.write_all(&data).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    let channels = reader.header().channels();
    assert_eq!(channels.len(), channel_count);
    assert_eq!(channels[0].name, "DAPI");
    assert_eq!(channels[0].color_rgb, [0, 0, 255]);
    assert!((channels[0].exposure_ms - 10.0).abs() < 1e-9);
    assert_eq!(channels[1].name, "FITC");
    assert_eq!(channels[1].color_rgb, [0, 255, 0]);
    assert!((channels[1].exposure_ms - 30.0).abs() < 1e-9);
}

// read_tile_bytes must return InvalidOffset rather than panicking on a
// negative data_length (guard against malformed/truncated sections).
#[test]
fn negative_data_length_returns_invalid_offset() {
    use std::io::Write as _;
    let mut f = NamedTempFile::new().unwrap();
    let header = common::make_header_section(common::HeaderSection::default());
    f.write_all(&header).unwrap();
    // Construct a raw tile section with data_length = -1 (i32 bytes: FF FF FF FF)
    // written at byte 32 of the section, bypassing make_tile_info_section.
    let section_start = header.len() as i64;
    // Build via helper with data_length=1, then patch the length field to -1.
    let mut raw = common::make_tile_info_section(0, 256, 256, 20.0, 1, -section_start);
    // data_length lives at byte 32 of the section.
    raw[32..36].copy_from_slice(&(-1i32).to_le_bytes());
    f.write_all(&raw).unwrap();
    f.flush().unwrap();

    let reader = KfbReader::open(f.path()).unwrap();
    assert_eq!(reader.tiles().len(), 1);
    let err = reader.read_tile_bytes(&reader.tiles()[0]).unwrap_err();
    assert!(
        matches!(err, KfbError::InvalidOffset { .. }),
        "expected InvalidOffset, got {err:?}"
    );
}
