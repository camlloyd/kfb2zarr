use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "kfb2zarr", version)]
#[command(about = "Convert KFBio whole slide images (.kfb, .kfbf) to OME-Zarr")]
struct Args {
    /// Input .kfb or .kfbf file
    input: PathBuf,
    /// Output .ome.zarr store
    output: PathBuf,
    /// Overwrite the output store if it already exists
    #[arg(long)]
    overwrite: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.output.exists() {
        if !args.overwrite {
            anyhow::bail!(
                "output path already exists: {} (pass --overwrite to replace it)",
                args.output.display()
            );
        }

        let metadata = fs::symlink_metadata(&args.output)?;
        if metadata.is_dir() {
            fs::remove_dir_all(&args.output)?;
        } else {
            fs::remove_file(&args.output)?;
        }
    }

    eprintln!(
        "Converting {} → {}",
        args.input.display(),
        args.output.display()
    );
    kfb2zarr::convert_to_zarr(&args.input, &args.output)?;
    eprintln!("Done.");
    Ok(())
}
