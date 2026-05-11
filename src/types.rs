/// Identifies the role of an associated image embedded in the slide file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociatedImageKind {
    /// Slide label image.
    Label,
    /// Secondary slide thumbnail image.
    Thumbnail,
}

/// Per-channel metadata parsed from a KFBF fluorescence file header.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMetadata {
    /// Channel name as recorded by the scanner (e.g. `"DAPI"`, `"FITC"`).
    pub name: String,
    /// Display color as an RGB triplet, each component 0–255.
    pub color_rgb: [u8; 3],
    /// Exposure time in milliseconds.
    pub exposure_ms: f64,
}

/// Metadata parsed from the fixed-size header at the start of a `.kfb` file.
#[derive(Debug, Clone)]
pub struct KfbHeader {
    format: KfbFormat,
    tile_count: i32,
    base_width: i32,
    base_height: i32,
    scan_scale: i32,
    spend_time: i32,
    scan_time: i64,
    image_cap_res: f64,
    tile_size: i32,
    channel_count: usize,
    pub(crate) zoom_levels: i32,
    channels: Vec<ChannelMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KfbFormat {
    Brightfield,
    Fluorescence,
}

#[derive(Debug, Clone)]
pub(crate) struct KfbHeaderFields {
    pub format: KfbFormat,
    pub tile_count: i32,
    pub base_width: i32,
    pub base_height: i32,
    pub scan_scale: i32,
    pub spend_time: i32,
    pub scan_time: i64,
    pub image_cap_res: f64,
    pub tile_size: i32,
    pub channel_count: usize,
    pub zoom_levels: i32,
    pub channels: Vec<ChannelMetadata>,
}

impl KfbHeader {
    pub(crate) fn new(fields: KfbHeaderFields) -> Self {
        debug_assert!(
            fields.channels.is_empty() || fields.channels.len() == fields.channel_count,
            "channels.len() ({}) must equal channel_count ({})",
            fields.channels.len(),
            fields.channel_count
        );
        Self {
            format: fields.format,
            tile_count: fields.tile_count,
            base_width: fields.base_width,
            base_height: fields.base_height,
            scan_scale: fields.scan_scale,
            spend_time: fields.spend_time,
            scan_time: fields.scan_time,
            image_cap_res: fields.image_cap_res,
            tile_size: fields.tile_size,
            channel_count: fields.channel_count,
            zoom_levels: fields.zoom_levels,
            channels: fields.channels,
        }
    }

    /// Total number of tile records encoded in the file.
    pub fn tile_count(&self) -> i32 {
        self.tile_count
    }
    pub(crate) fn is_fluorescence(&self) -> bool {
        self.format == KfbFormat::Fluorescence
    }
    /// Width of the full-resolution image in pixels.
    pub fn base_width(&self) -> i32 {
        self.base_width
    }
    /// Height of the full-resolution image in pixels.
    pub fn base_height(&self) -> i32 {
        self.base_height
    }
    /// Scan magnification setting (e.g. 20 for 20×).
    pub fn scan_scale(&self) -> i32 {
        self.scan_scale
    }
    /// Scan duration in seconds as recorded by the scanner.
    pub fn spend_time(&self) -> i32 {
        self.spend_time
    }
    /// Unix timestamp of the scan (seconds since 1970-01-01).
    pub fn scan_time(&self) -> i64 {
        self.scan_time
    }
    /// Nominal width and height of each tile in pixels.
    pub fn tile_size(&self) -> i32 {
        self.tile_size
    }
    /// Number of channels in the image.
    pub fn channel_count(&self) -> usize {
        self.channel_count
    }
    /// Number of resolution levels in the multi-resolution pyramid.
    pub fn zoom_levels(&self) -> i32 {
        self.zoom_levels
    }
    /// Physical pixel size in microns per pixel (MPP).
    pub fn mpp(&self) -> f64 {
        self.image_cap_res
    }
    /// Per-channel metadata (names, colors, exposure times) for fluorescence files.
    /// Empty for brightfield files or when the KFBF header lacks this block.
    pub fn channels(&self) -> &[ChannelMetadata] {
        &self.channels
    }
}

#[derive(Debug)]
pub(crate) struct DecodedAssociatedImage {
    pub kind: AssociatedImageKind,
    pub pixels: Vec<u8>,
    pub width: u64,
    pub height: u64,
}

/// Location and dimensions of a single JPEG-compressed tile within the file.
#[derive(Debug, Clone)]
pub struct TileInfo {
    pub(crate) pos_x: i32,
    pub(crate) pos_y: i32,
    width: i32,
    height: i32,
    channel_index: usize,
    pub(crate) zoom_level: i32,
    pub(crate) data_offset: u64,
    pub(crate) data_length: i32,
}

pub(crate) struct TileInfoFields {
    pub pos_x: i32,
    pub pos_y: i32,
    pub width: i32,
    pub height: i32,
    pub channel_index: usize,
    pub zoom_level: i32,
    pub data_offset: u64,
    pub data_length: i32,
}

impl TileInfo {
    pub(crate) fn new(
        pos_x: i32,
        pos_y: i32,
        width: i32,
        height: i32,
        zoom_level: i32,
        data_offset: u64,
        data_length: i32,
    ) -> Self {
        Self::from_fields(TileInfoFields {
            pos_x,
            pos_y,
            width,
            height,
            channel_index: 0,
            zoom_level,
            data_offset,
            data_length,
        })
    }

    pub(crate) fn from_fields(fields: TileInfoFields) -> Self {
        Self {
            pos_x: fields.pos_x,
            pos_y: fields.pos_y,
            width: fields.width,
            height: fields.height,
            channel_index: fields.channel_index,
            zoom_level: fields.zoom_level,
            data_offset: fields.data_offset,
            data_length: fields.data_length,
        }
    }

    /// X coordinate of the tile's top-left corner in the level's pixel space.
    pub fn pos_x(&self) -> i32 {
        self.pos_x
    }
    /// Y coordinate of the tile's top-left corner in the level's pixel space.
    pub fn pos_y(&self) -> i32 {
        self.pos_y
    }
    /// Width of this tile in pixels (may be smaller than `tile_size` at image edges).
    pub fn width(&self) -> i32 {
        self.width
    }
    /// Height of this tile in pixels (may be smaller than `tile_size` at image edges).
    pub fn height(&self) -> i32 {
        self.height
    }
    /// Channel index for fluorescence tiles. Brightfield RGB tiles use channel 0.
    pub fn channel_index(&self) -> usize {
        self.channel_index
    }
    /// Resolution level this tile belongs to, where 0 is full resolution.
    pub fn zoom_level(&self) -> i32 {
        self.zoom_level
    }
}

/// Metadata and file location of an associated image.
#[derive(Debug, Clone)]
pub struct AssociatedImage {
    kind: AssociatedImageKind,
    width: i32,
    height: i32,
    pub(crate) data_offset: u64,
    pub(crate) data_length: i32,
}

impl AssociatedImage {
    pub(crate) fn new(
        kind: AssociatedImageKind,
        width: i32,
        height: i32,
        data_offset: u64,
        data_length: i32,
    ) -> Self {
        Self {
            kind,
            width,
            height,
            data_offset,
            data_length,
        }
    }

    /// Whether this is a label or thumbnail image.
    pub fn kind(&self) -> AssociatedImageKind {
        self.kind
    }
    /// Width of the associated image in pixels.
    pub fn width(&self) -> i32 {
        self.width
    }
    /// Height of the associated image in pixels.
    pub fn height(&self) -> i32 {
        self.height
    }
}
