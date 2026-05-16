# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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