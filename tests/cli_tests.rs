use std::fs;
use std::process::Command;

use tempfile::TempDir;

fn kfb2zarr() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kfb2zarr"))
}

#[test]
fn refuses_existing_output_without_overwrite() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("missing.kfb");
    let output = dir.path().join("existing.ome.zarr");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("sentinel"), b"keep me").unwrap();

    let result = kfb2zarr().arg(&input).arg(&output).output().unwrap();

    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("output path already exists"));
    assert!(output.join("sentinel").exists());
}

#[test]
fn overwrite_removes_existing_output_file() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("missing.kfb");
    let output = dir.path().join("existing.ome.zarr");
    fs::write(&output, b"stale bytes").unwrap();

    let result = kfb2zarr()
        .arg("--overwrite")
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();

    assert!(!result.status.success());
    assert!(
        !output.exists(),
        "existing output file should be removed before input is opened"
    );
}

#[test]
fn overwrite_removes_existing_output_directory() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("missing.kfb");
    let output = dir.path().join("existing.ome.zarr");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("sentinel"), b"remove me").unwrap();

    let result = kfb2zarr()
        .arg("--overwrite")
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();

    assert!(!result.status.success());
    assert!(
        !output.exists(),
        "existing output should be removed before input is opened"
    );
}
