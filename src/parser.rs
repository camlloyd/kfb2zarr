use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

use crate::error::KfbError;
use crate::types::{
    AssociatedImage, AssociatedImageKind, ChannelMetadata, KfbFormat, KfbHeader, KfbHeaderFields,
    TileInfo,
};

const HEADER_START: [u8; 4] = [0xF1, 0x01, 0xEE, 0xEE];
const TILE_INFO_START: [u8; 4] = [0xF1, 0x04, 0xEE, 0xEE];

pub(crate) struct KfbfTileInfo {
    pub pos_x: i32,
    pub width: i32,
    pub height: i32,
    pub zoom_level: i32,
    pub offset_table: u64,
    pub length_table: u64,
}

pub(crate) fn parse_header(data: &[u8]) -> Result<KfbHeader, KfbError> {
    if data.len() < 4 || data[0..4] != HEADER_START {
        let mut actual = [0u8; 4];
        if data.len() >= 4 {
            actual.copy_from_slice(&data[0..4]);
        }
        return Err(KfbError::InvalidMagic {
            offset: 0,
            expected: HEADER_START,
            actual,
        });
    }

    let format = if data.get(4..8) == Some(b"KFBF") {
        KfbFormat::Fluorescence
    } else {
        KfbFormat::Brightfield
    };

    let mut cur = Cursor::new(&data[16..]);
    let tile_count = cur.read_i32::<LittleEndian>()?;
    let base_height = cur.read_i32::<LittleEndian>()?;
    let base_width = cur.read_i32::<LittleEndian>()?;
    let scan_scale = cur.read_i32::<LittleEndian>()?;
    let mut _codec = [0u8; 8];
    std::io::Read::read_exact(&mut cur, &mut _codec)?;
    let spend_time = cur.read_i32::<LittleEndian>()?;
    let scan_time = cur.read_i32::<LittleEndian>()? as i64;
    cur.set_position(0x4C - 0x10);
    let image_cap_res = cur.read_f32::<LittleEndian>()? as f64;
    cur.set_position(0x58 - 0x10);
    let kfb_tile_size = cur.read_i32::<LittleEndian>()?;

    let (base_width, base_height, channel_count, tile_size) = match format {
        KfbFormat::Brightfield => (base_width, base_height, 3, kfb_tile_size),
        // KFBF stores dimensions and tile size in different fixed-header slots.
        KfbFormat::Fluorescence => (
            base_height,
            base_width,
            parse_kfbf_channel_count(data),
            parse_kfbf_tile_size(data).unwrap_or(kfb_tile_size),
        ),
    };
    let max_dim = base_width.max(base_height) as f64;
    let zoom_levels = (max_dim.log2().ceil() as i32) + 1;

    let (channel_count, channels) = if format == KfbFormat::Fluorescence {
        parse_kfbf_channel_metadata(data, channel_count)
    } else {
        (channel_count, vec![])
    };

    Ok(KfbHeader::new(KfbHeaderFields {
        format,
        tile_count,
        base_width,
        base_height,
        scan_scale,
        spend_time,
        scan_time,
        image_cap_res,
        tile_size,
        channel_count,
        zoom_levels,
        channels,
    }))
}

fn parse_kfbf_channel_count(data: &[u8]) -> usize {
    const DEFAULT_CHANNELS: usize = 1;
    data.get(0xb0..0xb4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_be_bytes)
        .filter(|&channels| channels > 0 && channels < 32)
        .map_or(DEFAULT_CHANNELS, |channels| channels as usize)
}

/// Parse KFBF per-channel metadata from the TLV block at file offset 0xa8.
///
/// Returns `(channel_count, channels)`. `channel_count` is derived from the TLV
/// data block sizes when TLV entries are present (names block size / 40); otherwise
/// falls back to the `fallback_count` argument. `channels` is empty when any of the
/// three required tags is missing.
pub(crate) fn parse_kfbf_channel_metadata(
    data: &[u8],
    fallback_count: usize,
) -> (usize, Vec<ChannelMetadata>) {
    const TAG_NAMES: u32 = 0x4d;
    const TAG_COLORS: u32 = 0x4f;
    const TAG_EXPOSURES: u32 = 0x54;
    const TLV_SCAN_LIMIT: usize = 0x200;

    let mut names_offset: Option<usize> = None;
    let mut colors_offset: Option<usize> = None;
    let mut exposures_offset: Option<usize> = None;

    let mut pos = 0xa8usize;
    while let Some(tag_bytes) = data.get(pos..pos + 4) {
        let tag = u32::from_be_bytes(tag_bytes.try_into().unwrap());
        let len_bytes = match data.get(pos + 4..pos + 8) {
            Some(b) => b,
            None => break,
        };
        let len = u32::from_be_bytes(len_bytes.try_into().unwrap()) as usize;
        if tag == 0 || pos > TLV_SCAN_LIMIT {
            break;
        }
        if len == 8 {
            let val = match data.get(pos + 8..pos + 16) {
                Some(b) => b,
                None => break,
            };
            let offset = u32::from_le_bytes(val[3..7].try_into().unwrap()) as usize;
            match tag {
                TAG_NAMES => names_offset = Some(offset),
                TAG_COLORS => colors_offset = Some(offset),
                TAG_EXPOSURES => exposures_offset = Some(offset),
                _ => {}
            }
        }
        pos += 8 + len;
        if names_offset.is_some() && colors_offset.is_some() && exposures_offset.is_some() {
            break;
        }
    }

    let (names_off, colors_off, exposures_off) =
        match (names_offset, colors_offset, exposures_offset) {
            (Some(n), Some(c), Some(e)) => (n, c, e),
            _ => return (fallback_count, vec![]),
        };

    let channel_count = if colors_off > names_off {
        (colors_off - names_off) / 40
    } else {
        fallback_count
    };

    if channel_count == 0 {
        return (fallback_count, vec![]);
    }

    let names: Vec<String> = (0..channel_count)
        .map(|i| {
            let start = names_off + i * 40;
            let field = data.get(start..start + 40)?;
            let end = field.iter().position(|&b| b == 0).unwrap_or(40);
            std::str::from_utf8(&field[..end])
                .ok()
                .map(|s| s.to_owned())
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if names.len() != channel_count {
        return (fallback_count, vec![]);
    }

    let colors: Vec<[u8; 3]> = (0..channel_count)
        .map(|i| {
            let start = colors_off + i * 12;
            let field = data.get(start..start + 12)?;
            let r = u32::from_le_bytes(field[0..4].try_into().ok()?) as u8;
            let g = u32::from_le_bytes(field[4..8].try_into().ok()?) as u8;
            let b = u32::from_le_bytes(field[8..12].try_into().ok()?) as u8;
            Some([r, g, b])
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if colors.len() != channel_count {
        return (fallback_count, vec![]);
    }

    let exposures: Vec<f64> = (0..channel_count)
        .map(|i| {
            let start = exposures_off + i * 8;
            let field: [u8; 8] = data.get(start..start + 8)?.try_into().ok()?;
            Some(f64::from_le_bytes(field))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if exposures.len() != channel_count {
        return (fallback_count, vec![]);
    }

    let channels = names
        .into_iter()
        .zip(colors)
        .zip(exposures)
        .map(|((name, color_rgb), exposure_ms)| ChannelMetadata {
            name,
            color_rgb,
            exposure_ms,
        })
        .collect();

    (channel_count, channels)
}

fn parse_kfbf_tile_size(data: &[u8]) -> Option<i32> {
    data.get(0x58..0x5c)
        .and_then(|bytes| bytes.try_into().ok())
        .map(i32::from_le_bytes)
        .filter(|&tile_size| tile_size > 0)
}

pub(crate) fn parse_associated_image(
    data: &[u8],
    kind: AssociatedImageKind,
    section_offset: u64,
) -> Result<AssociatedImage, KfbError> {
    let mut cur = Cursor::new(&data[8..]);
    let height = cur.read_i32::<LittleEndian>()?;
    let width = cur.read_i32::<LittleEndian>()?;
    let _samples_per_pixel = cur.read_i32::<LittleEndian>()?;
    let data_length = cur.read_i32::<LittleEndian>()?;

    Ok(AssociatedImage::new(
        kind,
        width,
        height,
        section_offset + 52,
        data_length,
    ))
}

pub(crate) fn parse_tile_info(
    data: &[u8],
    section_start: u64,
    max_mag: f32,
) -> Result<TileInfo, KfbError> {
    if data.len() < 4 || data[0..4] != TILE_INFO_START {
        let mut actual = [0u8; 4];
        if data.len() >= 4 {
            actual.copy_from_slice(&data[0..4]);
        }
        return Err(KfbError::InvalidMagic {
            offset: section_start,
            expected: TILE_INFO_START,
            actual,
        });
    }

    let mut cur = Cursor::new(&data[8..]);
    // KFB tile section layout (confirmed by field analysis):
    //   byte  8-11: y-coord at native resolution
    //   byte 12-15: tile width in pixels
    //   byte 16-19: tile height in pixels
    //   byte 20-23: zoom magnification as f32 (20.0 = full resolution)
    //   byte 24-27: unused (always 0)
    // pos_x is left as 0 and set later in KfbReader::scan() from section rank.
    let y_native = cur.read_i32::<LittleEndian>()?;
    let tile_w = cur.read_i32::<LittleEndian>()?;
    let tile_h = cur.read_i32::<LittleEndian>()?;
    let mag = cur.read_f32::<LittleEndian>()?;
    let _unused = cur.read_i32::<LittleEndian>()?;
    // skip 4 bytes
    let pos = cur.position();
    cur.set_position(pos + 4);
    let data_length = cur.read_i32::<LittleEndian>()?;
    let offset_from_file = cur.read_i64::<LittleEndian>()?;

    // zoom_level 0 = full resolution; each step halves magnification.
    let zoom_level = if mag > 0.0 && max_mag > 0.0 {
        (max_mag / mag).log2().round() as i32
    } else {
        0
    };

    Ok(TileInfo::new(
        0,
        y_native,
        tile_w,
        tile_h,
        zoom_level,
        (section_start as i64 + offset_from_file) as u64,
        data_length,
    ))
}

pub(crate) fn parse_kfbf_tile_info(
    data: &[u8],
    section_start: u64,
    max_mag: f32,
) -> Result<KfbfTileInfo, KfbError> {
    if data.len() < 64 || data[0..4] != TILE_INFO_START {
        let mut actual = [0u8; 4];
        if data.len() >= 4 {
            actual.copy_from_slice(&data[0..4]);
        }
        return Err(KfbError::InvalidMagic {
            offset: section_start,
            expected: TILE_INFO_START,
            actual,
        });
    }

    let mut cur = Cursor::new(&data[8..]);
    let pos_x = cur.read_i32::<LittleEndian>()?;
    let tile_h = cur.read_i32::<LittleEndian>()?;
    let tile_w = cur.read_i32::<LittleEndian>()?;
    let mag = cur.read_f32::<LittleEndian>()?;
    cur.set_position(24);
    let _data_length = cur.read_i32::<LittleEndian>()?;
    let offset_table = cur.read_i64::<LittleEndian>()? as u64;
    let length_table = cur.read_i64::<LittleEndian>()? as u64;

    let zoom_level = if mag > 0.0 && max_mag > 0.0 {
        (max_mag / mag).log2().round() as i32
    } else {
        0
    };

    Ok(KfbfTileInfo {
        pos_x,
        width: tile_w,
        height: tile_h,
        zoom_level,
        offset_table,
        length_table,
    })
}
