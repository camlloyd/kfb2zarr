# kfb2zarr

[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.20101669.svg)](https://doi.org/10.5281/zenodo.20101669)

Convert [KFBio](https://www.kfbio.cn) whole slide images (`.kfb`, `.kfbf`) to [OME-Zarr](https://ngff.openmicroscopy.org/).

## Installation

via cargo:

```sh
cargo install kfb2zarr
```

via conda:
```sh
conda install bioconda::kfb2zarr
```

## Usage

```sh
kfb2zarr slide.kfb slide.ome.zarr
```

To replace an existing output store:

```sh
kfb2zarr slide.kfb slide.ome.zarr --overwrite
```

To limit CPU usage (default uses all cores):

```sh
kfb2zarr slide.kfb slide.ome.zarr --threads 4
```

## Viewing the output

The output `.ome.zarr` can be viewed locally with:

- **[QuPath](https://qupath.github.io/)** ≥ 0.7.0 via drag & drop
- **[napari](https://napari.org/)** via the [napari-ome-zarr](https://github.com/ome/napari-ome-zarr) plugin

## Acknowledgements

Sample data used during development was kindly provided by KFBio.
kfb2zarr is not an official product of KFBio.

kfb2zarr writes Zarr stores with:

Deakin, L. (2025). *zarrs: A High-Performance Rust Library for the Zarr Array Storage Format* (Version 0.22.10) [Computer software]. https://github.com/zarrs/zarrs

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

