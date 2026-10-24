# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/camlloyd/kfb2zarr/compare/v0.1.3...v0.2.0) - 2026-10-24

### Added

- *(reader)* read macro and real thumbnail images
- *(cli)* add --chunk-size option

### Fixed

- *(reader)* return errors instead of panicking on damaged headers
- *(reader)* read tile x positions from tile records
- *(parser)* read KFBF width and height from the same slots as KFB
- *(deps)* bump crossbeam-epoch to v0.9.20

### Other

- *(types)* [**breaking**] remove KfbHeader::zoom_levels
- *(api)* document error fields and cases
- *(types)* [**breaking**] make public enums `non_exhaustive`
- *(modules)* make internal modules crate-private
- *(reader)* add validation tests for tile index errors
- *(zarr)* build channel names and colors in one place
- work out pyramid levels in one module
- *(reader)* walk the tile index for both formats
- *(deps)* drop zune-core
- *(deps)* bump thiserror to v2
- update docstrings
- bump actions/checkout to v6.0.3
- *(citation)* use single license for Zenodo
- *(readme)* use Concept DOI
- *(zarr)* use ome_zarr_metadata
- *(zarr)* use zstd@1 as default compressor
- replace nested loops with copy_from_slice

## [0.1.3](https://github.com/camlloyd/kfb2zarr/compare/v0.1.2...v0.1.3) - 2026-05-16

### Fixed

- derive all pyramid level shapes from level 0
- *(parser)* swap fluorescence tile x/y
- *(parser)* compute zoom_levels from tile size

### Other

- *(readme)* move acknowledgements section
- *(cargo)* adopt dual MIT OR Apache-2.0 license
- *(citation)* update Zenodo DOI

## [0.1.2](https://github.com/camlloyd/kfb2zarr/compare/v0.1.1...v0.1.2) - 2026-05-11

### Added

- *(cli)* add --threads option

### Other

- *(readme)* add --threads option usage
- *(zarr)* decode tiles on demand
- *(citation)* add Zenodo DOI

## [0.1.1](https://github.com/camlloyd/kfb2zarr/compare/v0.1.0...v0.1.1) - 2026-05-10

### Fixed

- *(parser)* read codec label as 8 bytes

### Other

- *(zarr)* use CARGO_PKG_VERSION
- *(citation)* correct CITATION.cff
- *(readme)* remove outdated spot-check note
- *(readme)* add conda install instructions

## [0.1.0] - 2026-05-09

### Added
- Initial release