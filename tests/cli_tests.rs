use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

// Helper to create a source directory with a specific structure for testing.
fn create_test_src(tmp_dir: &TempDir, structure: &[(&str, &[u8])]) -> PathBuf {
    let source = tmp_dir.path().join("source");
    fs::create_dir_all(&source).unwrap();
    for (path, content) in structure {
        let full_path = source.join(path);
        if path.ends_with('/') {
            fs::create_dir_all(full_path).unwrap();
        } else {
            fs::create_dir_all(full_path.parent().unwrap()).unwrap();
            let mut file = File::create(full_path).unwrap();
            file.write_all(content).unwrap();
        }
    }
    source
}

// Helper to verify that the contents of two directories are identical.
fn assert_dirs_equal(dir1: &Path, dir2: &Path) {
    let mut paths1 = HashSet::new();
    for entry in walkdir::WalkDir::new(dir1) {
        let entry = entry.unwrap();
        if entry.path() == dir1 {
            continue;
        }
        let path = entry.path().strip_prefix(dir1).unwrap().to_path_buf();
        paths1.insert(path);
    }

    let mut paths2 = HashSet::new();
    for entry in walkdir::WalkDir::new(dir2) {
        let entry = entry.unwrap();
        if entry.path() == dir2 {
            continue;
        }
        let path = entry.path().strip_prefix(dir2).unwrap().to_path_buf();
        paths2.insert(path);
    }

    assert_eq!(paths1, paths2, "Directory structures are not equal");
}

#[test]
fn test_copy_single_file_succeeds() {
    let tmp_dir = TempDir::new().unwrap();
    let source_path = tmp_dir.path().join("source");
    fs::create_dir(&source_path).unwrap();
    let dest_path = tmp_dir.path().join("dest");
    fs::create_dir(&dest_path).unwrap();

    let file_to_copy = source_path.join("file.txt");
    File::create(&file_to_copy).unwrap().write_all(b"content").unwrap();

    let mut cmd = Command::cargo_bin("cp2").unwrap();
    cmd.arg(file_to_copy).arg(&dest_path);

    cmd.assert().success();

    let expected_dest_file = dest_path.join("file.txt");
    assert!(expected_dest_file.exists());
    assert_eq!(fs::read_to_string(expected_dest_file).unwrap(), "content");
}

#[test]
fn test_copy_dir_without_recursive_flag_fails() {
    let tmp_dir = TempDir::new().unwrap();
    let source_path = tmp_dir.path().join("source");
    fs::create_dir(&source_path).unwrap();
    let dest_path = tmp_dir.path().join("dest");
    fs::create_dir(&dest_path).unwrap();

    let mut cmd = Command::cargo_bin("cp2").unwrap();
    cmd.arg(&source_path).arg(&dest_path);

    cmd.assert().failure().stderr(predicate::str::contains(
        "Source path is a directory, but recursive flag is not set",
    ));
}

#[test]
fn test_copy_dir_with_recursive_flag_succeeds() {
    let tmp_dir = TempDir::new().unwrap();
    let dest_path = tmp_dir.path().join("dest");
    fs::create_dir(&dest_path).unwrap();

    let source_path = create_test_src(&tmp_dir, &[("f1.txt", b"1"), ("sub/f2.txt", b"2")]);

    let mut cmd = Command::cargo_bin("cp2").unwrap();
    cmd.arg("-r").arg(&source_path).arg(&dest_path);

    cmd.assert().success();

    let expected_dest = dest_path.join("source");
    assert_dirs_equal(&source_path, &expected_dest);
}

#[test]
fn test_non_existent_source_fails() {
    let tmp_dir = TempDir::new().unwrap();
    let dest_path = tmp_dir.path().join("dest");
    fs::create_dir(&dest_path).unwrap();

    let mut cmd = Command::cargo_bin("cp2").unwrap();
    cmd.arg("non-existent-file").arg(&dest_path);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Source path does not exist"));
}

#[test]
fn test_quiet_mode_has_no_stdout() {
    let tmp_dir = TempDir::new().unwrap();
    let source_path = tmp_dir.path().join("source");
    fs::create_dir(&source_path).unwrap();
    let dest_path = tmp_dir.path().join("dest");
    fs::create_dir(&dest_path).unwrap();

    let file_to_copy = source_path.join("file.txt");
    File::create(&file_to_copy).unwrap().write_all(b"content").unwrap();

    let mut cmd = Command::cargo_bin("cp2").unwrap();
    cmd.arg("-q").arg(file_to_copy).arg(&dest_path);

    // In quiet mode, there should be no output to stdout (like "File copied successfully!")
    // and no progress bar on stderr.
    cmd.assert().success().stdout(predicate::str::is_empty());
}
