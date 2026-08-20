//! Read KFBio whole-slide image files (`.kfb`, `.kfbf`) and convert them to OME-Zarr v0.4 (Zarr v2).
//!
//! Two entry points:
//!
//! - [`KfbReader`] — opens a KFBio file and exposes its header, tile index, and
//!   associated images.
//! - [`convert_to_zarr`] — decodes JPEG tiles on demand, level by level in
//!   parallel, and writes a multi-resolution OME-Zarr v0.4 store.
//!
//! Pure Rust, no C dependencies. JPEG decoding via [`zune_jpeg`](https://docs.rs/zune-jpeg).
//! Physical coordinate transforms come from the MPP value in the file header.
//!
//! # Examples
//!
//! ## Convert a KFBio file to OME-Zarr
//!
//! ```no_run
//! use std::path::Path;
//! use kfb2zarr::convert_to_zarr;
//!
//! convert_to_zarr(Path::new("slide.kfb"), Path::new("slide.ome.zarr"))?;
//! # Ok::<(), kfb2zarr::KfbError>(())
//! ```
//!
//! ## Inspect metadata without converting
//!
//! ```no_run
//! use std::path::Path;
//! use kfb2zarr::KfbReader;
//!
//! let reader = KfbReader::open(Path::new("slide.kfb"))?;
//! let header = reader.header();
//! println!("dimensions: {}x{}", header.base_width(), header.base_height());
//! println!("mpp: {}", header.mpp());
//! println!("tiles: {}", reader.tiles().len());
//! # Ok::<(), kfb2zarr::KfbError>(())
//! ```

pub(crate) mod convert;
pub(crate) mod decode;
pub mod error;
pub(crate) mod parser;
pub mod reader;
pub mod types;
pub(crate) mod zarr;

pub use convert::convert_to_zarr;
pub use error::KfbError;
pub use reader::KfbReader;
pub use types::{AssociatedImage, AssociatedImageKind, ChannelMetadata, KfbHeader, TileInfo};
