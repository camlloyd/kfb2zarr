# kfb2zarr

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

## Viewing the output

The output `.ome.zarr` can be viewed locally with:

- **[QuPath](https://qupath.github.io/)** ≥ 0.7.0 via drag & drop
- **[napari](https://napari.org/)** via the [napari-ome-zarr](https://github.com/ome/napari-ome-zarr) plugin

## Acknowledgements

Sample data used during development was kindly provided by KFBio.
kfb2zarr is not an official product of KFBio.

kfb2zarr writes Zarr stores with:

Deakin, L. (2025). *zarrs: A High-Performance Rust Library for the Zarr Array Storage Format* (Version 0.22.10) [Computer software]. https://github.com/zarrs/zarrs
