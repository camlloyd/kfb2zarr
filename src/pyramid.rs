use crate::types::{KfbHeader, TileInfo};

/// One resolution of the pyramid, with its size in pixels and the tiles that cover it.
pub(crate) struct Level {
    pub width: u64,
    pub height: u64,
    pub tiles: Vec<TileInfo>,
}

/// A slide's levels, worked out from its tile records.
pub(crate) struct Pyramid {
    /// Width and height of a full tile: the largest level-0 tile record, or the
    /// header's nominal tile size when the slide has no level-0 tiles.
    pub tile_size: u64,
    pub levels: Vec<Level>,
}

impl Pyramid {
    pub(crate) fn new(header: &KfbHeader, tiles: &[TileInfo]) -> Self {
        let tiles: Vec<&TileInfo> = tiles.iter().filter(|tile| tile.data_length > 0).collect();
        let tile_size = tiles
            .iter()
            .filter(|tile| tile.zoom_level() == 0)
            .map(|tile| tile.width().max(tile.height()))
            .max()
            .unwrap_or(header.tile_size()) as u64;

        // Count multi-tile levels, then add 1 for the single-tile level.
        let max_dim = header.base_width().max(header.base_height()) as f64;
        let tile_levels = (max_dim / tile_size.max(1) as f64).log2().ceil().max(0.0);
        let level_count = tile_levels as usize + 1;

        let mut by_level: Vec<Vec<TileInfo>> = vec![Vec::new(); level_count];
        for tile in tiles {
            let level = tile.zoom_level() as usize;
            if level < level_count {
                by_level[level].push(tile.clone());
            }
        }

        // Level 0 covers the header's size and every level-0 tile; each further
        // level halves it, rounding up.
        let header_size = (header.base_width() as u64, header.base_height() as u64);
        let (width, height) = by_level[0].iter().fold(header_size, |(w, h), tile| {
            let right = (tile.pos_x() + tile.width()).max(0) as u64;
            let bottom = (tile.pos_y() + tile.height()).max(0) as u64;
            (w.max(right), h.max(bottom))
        });

        let levels = by_level
            .into_iter()
            .enumerate()
            .map(|(i, tiles)| Level {
                width: width.div_ceil(1 << i),
                height: height.div_ceil(1 << i),
                tiles,
            })
            .collect();
        Self { tile_size, levels }
    }
}
