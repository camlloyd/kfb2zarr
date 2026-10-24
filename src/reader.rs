use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

use crate::error::KfbError;
use crate::parser::{parse_associated_image, parse_header, parse_tile_record};
use crate::types::{AssociatedImage, AssociatedImageKind, KfbHeader, TileInfo, TileInfoFields};

const TILE_INFO_START: [u8; 4] = [0xF1, 0x04, 0xEE, 0xEE];
const THUMBNAIL_START: [u8; 4] = [0xF1, 0x02, 0xEE, 0xEE];
const LABEL_START: [u8; 4] = [0xF1, 0x03, 0xEE, 0xEE];

/// Memory-mapped reader for a `.kfb` whole-slide image file.
///
/// Open a file with [`KfbReader::open`], then use [`header`](KfbReader::header),
/// [`tiles`](KfbReader::tiles), and [`associated_images`](KfbReader::associated_images)
/// to inspect its contents, or call [`read_tile_bytes`](KfbReader::read_tile_bytes) or
/// [`read_associated_bytes`](KfbReader::read_associated_bytes) to retrieve raw JPEG data.
pub struct KfbReader {
    // Mmap does not implement Debug, so we implement it manually below.
    mmap: Mmap,
    header: KfbHeader,
    tiles: Vec<TileInfo>,
    associated: Vec<AssociatedImage>,
}

impl KfbReader {
    /// Open a `.kfb` file at `path`, memory-map it, and parse its header and index.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// use kfb2zarr::KfbReader;
    ///
    /// let reader = KfbReader::open(Path::new("slide.kfb"))?;
    /// println!("{}x{} px at {:.4} µm/px",
    ///     reader.header().base_width(),
    ///     reader.header().base_height(),
    ///     reader.header().mpp());
    /// # Ok::<(), kfb2zarr::KfbError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`KfbError::Io`] if the file cannot be opened or mapped, or is too short
    /// to hold a header or tile record, [`KfbError::InvalidMagic`] if the file does not
    /// contain valid KFB magic bytes, and [`KfbError::InvalidOffset`] if the tile index
    /// points outside the file.
    pub fn open(path: &Path) -> Result<Self, KfbError> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        let (header, tiles, associated) = Self::scan(&mmap)?;
        Ok(Self {
            mmap,
            header,
            tiles,
            associated,
        })
    }

    fn scan(data: &[u8]) -> Result<(KfbHeader, Vec<TileInfo>, Vec<AssociatedImage>), KfbError> {
        let header = parse_header(data)?;
        let tiles = Self::scan_tiles(data, &header)?;
        let associated = Self::scan_associated_images(data, &tiles)?;
        Ok((header, tiles, associated))
    }

    fn scan_associated_images(
        data: &[u8],
        tiles: &[TileInfo],
    ) -> Result<Vec<AssociatedImage>, KfbError> {
        let tile_ranges = tile_payload_ranges(tiles);
        let thumbnail_offset = read_u64_le(data, 0x3c)?;
        let mut associated = Vec::new();
        let mut i = 0usize;
        let mut range_index = 0usize;
        while i + 56 <= data.len() {
            while range_index < tile_ranges.len() && tile_ranges[range_index].1 <= i as u64 {
                range_index += 1;
            }
            if let Some(&(start, end)) = tile_ranges.get(range_index) {
                if i as u64 >= start && (i as u64) < end {
                    i = end as usize;
                    continue;
                }
            }
            let marker = &data[i..i + 4];
            let kind = if marker == LABEL_START {
                AssociatedImageKind::Label
            } else if marker == THUMBNAIL_START {
                // The macro and the thumbnail share this marker. Header offset 0x3c
                // points at the thumbnail.
                if i as u64 == thumbnail_offset {
                    AssociatedImageKind::Thumbnail
                } else {
                    AssociatedImageKind::Macro
                }
            } else {
                i += 1;
                continue;
            };
            if associated
                .iter()
                .any(|a: &AssociatedImage| a.kind() == kind)
            {
                i += 1;
                continue;
            }
            let img = parse_associated_image(&data[i..], kind, i as u64)?;
            if let Some(img) = normalize_associated_image_jpeg_length(data, img, i + 52) {
                i += 52 + img.data_length.max(0) as usize;
                associated.push(img);
                continue;
            }
            i += 1;
        }
        Ok(associated)
    }

    fn scan_tiles(data: &[u8], header: &KfbHeader) -> Result<Vec<TileInfo>, KfbError> {
        let max_mag = header.scan_scale() as f32;
        let channel_count = header.channel_count();
        let mut tiles = Vec::new();
        let tile_index_offset = read_u64_le(data, 0x44)? as usize;

        for index in 0..header.tile_count().max(0) as usize {
            let i = tile_index_offset + index * 64;
            let end = i.checked_add(64).ok_or(KfbError::InvalidOffset {
                offset: i as u64,
                file_len: data.len() as u64,
            })?;
            if end > data.len() {
                return Err(KfbError::InvalidOffset {
                    offset: i as u64,
                    file_len: data.len() as u64,
                });
            }

            let record = parse_tile_record(&data[i..end], i as u64, max_mag)?;
            if !header.is_fluorescence() {
                tiles.push(TileInfo::from_fields(TileInfoFields {
                    pos_x: record.pos_x,
                    pos_y: record.pos_y,
                    width: record.width,
                    height: record.height,
                    channel_index: 0,
                    zoom_level: record.zoom_level,
                    data_offset: (tile_index_offset as i64 + record.data_offset) as u64,
                    data_length: record.data_length,
                }));
                continue;
            }

            for channel_index in 0..channel_count {
                let offset_table_pos = record.data_offset as u64 + (channel_index as u64 * 8);
                let length_table_pos = record.length_table + (channel_index as u64 * 8);
                let data_offset = read_u64_le(data, offset_table_pos)?;
                let data_length = read_u64_le(data, length_table_pos)?;
                let data_length =
                    i32::try_from(data_length).map_err(|_| KfbError::InvalidOffset {
                        offset: length_table_pos,
                        file_len: data.len() as u64,
                    })?;
                tiles.push(TileInfo::from_fields(TileInfoFields {
                    pos_x: record.pos_x,
                    pos_y: record.pos_y,
                    width: record.width,
                    height: record.height,
                    channel_index,
                    zoom_level: record.zoom_level,
                    data_offset,
                    data_length,
                }));
            }
        }

        Ok(tiles)
    }

    /// Return the parsed file header containing image dimensions, MPP, and tile geometry.
    pub fn header(&self) -> &KfbHeader {
        &self.header
    }

    /// Return all tile descriptors found in the file.
    pub fn tiles(&self) -> &[TileInfo] {
        &self.tiles
    }

    /// Return the associated images embedded in the file.
    pub fn associated_images(&self) -> &[AssociatedImage] {
        &self.associated
    }
}

fn read_u64_le(data: &[u8], offset: u64) -> Result<u64, KfbError> {
    let start = offset as usize;
    let end = start.checked_add(8).ok_or(KfbError::InvalidOffset {
        offset,
        file_len: data.len() as u64,
    })?;
    if end > data.len() {
        return Err(KfbError::InvalidOffset {
            offset,
            file_len: data.len() as u64,
        });
    }
    Ok(u64::from_le_bytes(data[start..end].try_into().unwrap()))
}

fn tile_payload_ranges(tiles: &[TileInfo]) -> Vec<(u64, u64)> {
    let mut ranges: Vec<_> = tiles
        .iter()
        .filter_map(|tile| {
            let length = u64::try_from(tile.data_length).ok()?;
            let end = tile.data_offset.checked_add(length)?;
            Some((tile.data_offset, end))
        })
        .collect();
    ranges.sort_unstable_by_key(|&(start, _)| start);
    ranges
}

fn normalize_associated_image_jpeg_length(
    data: &[u8],
    image: AssociatedImage,
    search_start: usize,
) -> Option<AssociatedImage> {
    let Ok(start) = usize::try_from(image.data_offset) else {
        return None;
    };
    if data.get(start..start + 2) != Some(&[0xFF, 0xD8]) {
        return None;
    };

    let declared_end = usize::try_from(image.data_length)
        .ok()
        .and_then(|len| start.checked_add(len));
    if let Some(end) = declared_end {
        if let Some(payload) = data.get(start..end) {
            if payload.len() >= 4 && payload.ends_with(&[0xFF, 0xD9]) {
                return Some(image);
            }
        }
    }

    let search_start = search_start.max(start);
    let search_end = next_kfb_marker(data, search_start).unwrap_or(data.len());
    let payload = data.get(start..search_end)?;
    let eoi = payload
        .windows(2)
        .position(|bytes| bytes == [0xFF, 0xD9])
        .map(|pos| pos + 2)?;
    let data_length = i32::try_from(eoi).ok()?;
    if data_length < 4 {
        return None;
    };

    Some(AssociatedImage::new(
        image.kind(),
        image.width(),
        image.height(),
        image.data_offset,
        data_length,
    ))
}

fn next_kfb_marker(data: &[u8], start: usize) -> Option<usize> {
    data.get(start..)?
        .windows(4)
        .position(|bytes| {
            bytes == THUMBNAIL_START
                || bytes == LABEL_START
                || bytes == TILE_INFO_START
                || bytes == [0xF1, 0x01, 0xEE, 0xEE]
                || bytes == [0xFF, 0x01, 0xEE, 0xEE]
        })
        .map(|offset| start + offset)
}

impl std::fmt::Debug for KfbReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KfbReader")
            .field("header", &self.header)
            .field("tiles", &format_args!("[{} tiles]", self.tiles.len()))
            .field("associated", &self.associated)
            .finish()
    }
}

impl KfbReader {
    /// Return the raw JPEG-compressed bytes for `tile` as a slice into the memory map.
    ///
    /// # Errors
    ///
    /// Returns [`KfbError::InvalidOffset`] if the tile's recorded data range extends
    /// beyond the end of the file.
    pub fn read_tile_bytes(&self, tile: &TileInfo) -> Result<&[u8], KfbError> {
        let start = tile.data_offset as usize;
        let len = usize::try_from(tile.data_length).map_err(|_| KfbError::InvalidOffset {
            offset: tile.data_offset,
            file_len: self.mmap.len() as u64,
        })?;
        let end = start.checked_add(len).ok_or(KfbError::InvalidOffset {
            offset: tile.data_offset,
            file_len: self.mmap.len() as u64,
        })?;
        if end > self.mmap.len() {
            return Err(KfbError::InvalidOffset {
                offset: tile.data_offset,
                file_len: self.mmap.len() as u64,
            });
        }
        Ok(&self.mmap[start..end])
    }

    /// Return the raw JPEG-compressed bytes for `img` as a slice into the memory map.
    ///
    /// # Errors
    ///
    /// Returns [`KfbError::InvalidOffset`] if the image's recorded data range extends
    /// beyond the end of the file.
    pub fn read_associated_bytes(&self, img: &AssociatedImage) -> Result<&[u8], KfbError> {
        let start = img.data_offset as usize;
        let len = usize::try_from(img.data_length).map_err(|_| KfbError::InvalidOffset {
            offset: img.data_offset,
            file_len: self.mmap.len() as u64,
        })?;
        let end = start.checked_add(len).ok_or(KfbError::InvalidOffset {
            offset: img.data_offset,
            file_len: self.mmap.len() as u64,
        })?;
        if end > self.mmap.len() {
            return Err(KfbError::InvalidOffset {
                offset: img.data_offset,
                file_len: self.mmap.len() as u64,
            });
        }
        Ok(&self.mmap[start..end])
    }
}
