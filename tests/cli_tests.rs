use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::cargo_bin;
use predicates::prelude::*;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
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

    let mut cmd = Command::new(cargo_bin!("cp2"));
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

    let mut cmd = Command::new(cargo_bin!("cp2"));
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

    let mut cmd = Command::new(cargo_bin!("cp2"));
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

    let mut cmd = Command::new(cargo_bin!("cp2"));
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

    let mut cmd = Command::new(cargo_bin!("cp2"));
    cmd.arg("-q").arg(file_to_copy).arg(&dest_path);

    // In quiet mode, there should be no output to stdout (like "File copied successfully!")
    // and no progress bar on stderr.
    cmd.assert().success().stdout(predicate::str::is_empty());
}

#[test]
fn test_copy_empty_file() {
    let tmp_dir = TempDir::new().unwrap();
    let dest_path = tmp_dir.path().join("dest");
    fs::create_dir(&dest_path).unwrap();

    let file = tmp_dir.path().join("empty.txt");
    File::create(&file).unwrap();

    Command::new(cargo_bin!("cp2"))
        .arg(&file)
        .arg(&dest_path)
        .assert()
        .success();

    let dest_file = dest_path.join("empty.txt");
    assert!(dest_file.exists());
    assert_eq!(fs::metadata(&dest_file).unwrap().len(), 0);
}

#[test]
fn test_copy_multiple_files() {
    let tmp_dir = TempDir::new().unwrap();
    let src = tmp_dir.path().join("src");
    fs::create_dir(&src).unwrap();
    let dest = tmp_dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    File::create(src.join("a.txt")).unwrap().write_all(b"aaa").unwrap();
    File::create(src.join("b.txt")).unwrap().write_all(b"bbb").unwrap();
    File::create(src.join("c.txt")).unwrap().write_all(b"ccc").unwrap();

    Command::new(cargo_bin!("cp2"))
        .arg(src.join("a.txt"))
        .arg(src.join("b.txt"))
        .arg(src.join("c.txt"))
        .arg(&dest)
        .assert()
        .success();

    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "aaa");
    assert_eq!(fs::read_to_string(dest.join("b.txt")).unwrap(), "bbb");
    assert_eq!(fs::read_to_string(dest.join("c.txt")).unwrap(), "ccc");
}

#[test]
fn test_copy_file_with_spaces_in_name() {
    let tmp_dir = TempDir::new().unwrap();
    let dest = tmp_dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    let file = tmp_dir.path().join("my file name.txt");
    File::create(&file).unwrap().write_all(b"spaced").unwrap();

    Command::new(cargo_bin!("cp2"))
        .arg(&file)
        .arg(&dest)
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(dest.join("my file name.txt")).unwrap(),
        "spaced"
    );
}

#[test]
fn test_copy_deeply_nested_directory() {
    let tmp_dir = TempDir::new().unwrap();
    let dest = tmp_dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    let source = create_test_src(
        &tmp_dir,
        &[
            ("a/b/c/d/deep.txt", b"deep"),
            ("a/b/sibling.txt", b"sibling"),
            ("top.txt", b"top"),
        ],
    );

    Command::new(cargo_bin!("cp2"))
        .arg("-r")
        .arg(&source)
        .arg(&dest)
        .assert()
        .success();

    let copied = dest.join("source");
    assert_eq!(
        fs::read_to_string(copied.join("a/b/c/d/deep.txt")).unwrap(),
        "deep"
    );
    assert_eq!(
        fs::read_to_string(copied.join("a/b/sibling.txt")).unwrap(),
        "sibling"
    );
    assert_eq!(fs::read_to_string(copied.join("top.txt")).unwrap(), "top");
}

#[test]
fn test_copy_empty_directory_recursive() {
    let tmp_dir = TempDir::new().unwrap();
    let dest = tmp_dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    let source = create_test_src(&tmp_dir, &[("empty_sub/", b"")]);

    Command::new(cargo_bin!("cp2"))
        .arg("-r")
        .arg(&source)
        .arg(&dest)
        .assert()
        .success();

    let copied = dest.join("source");
    assert!(copied.exists());
    assert!(copied.join("empty_sub").exists());
    assert!(copied.join("empty_sub").is_dir());
}

#[test]
fn test_copy_with_sync_flag() {
    let tmp_dir = TempDir::new().unwrap();
    let dest = tmp_dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    let file = tmp_dir.path().join("sync_test.txt");
    File::create(&file).unwrap().write_all(b"synced data").unwrap();

    Command::new(cargo_bin!("cp2"))
        .arg("-S")
        .arg(&file)
        .arg(&dest)
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(dest.join("sync_test.txt")).unwrap(),
        "synced data"
    );
}

#[test]
fn test_copy_preserves_binary_content() {
    let tmp_dir = TempDir::new().unwrap();
    let dest = tmp_dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    // Write binary content with all byte values
    let binary_data: Vec<u8> = (0..=255).collect();
    let file = tmp_dir.path().join("binary.bin");
    File::create(&file).unwrap().write_all(&binary_data).unwrap();

    Command::new(cargo_bin!("cp2"))
        .arg(&file)
        .arg(&dest)
        .assert()
        .success();

    let mut result = Vec::new();
    File::open(dest.join("binary.bin"))
        .unwrap()
        .read_to_end(&mut result)
        .unwrap();
    assert_eq!(result, binary_data);
}

#[test]
fn test_partial_failure_copies_valid_sources() {
    let tmp_dir = TempDir::new().unwrap();
    let dest = tmp_dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    let file = tmp_dir.path().join("good.txt");
    File::create(&file).unwrap().write_all(b"good").unwrap();

    // One valid file, one nonexistent — should still copy the valid one
    Command::new(cargo_bin!("cp2"))
        .arg(&file)
        .arg("nonexistent.txt")
        .arg(&dest)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Source path does not exist"));

    assert_eq!(fs::read_to_string(dest.join("good.txt")).unwrap(), "good");
}

#[test]
fn test_non_existent_destination_fails() {
    let tmp_dir = TempDir::new().unwrap();
    let file = tmp_dir.path().join("file.txt");
    File::create(&file).unwrap().write_all(b"data").unwrap();

    Command::new(cargo_bin!("cp2"))
        .arg(&file)
        .arg(tmp_dir.path().join("no_such_dest"))
        .assert()
        .failure()
        .stdout(predicate::str::contains("Destination path does not exist"));
}

#[test]
fn test_copy_with_custom_parallelism() {
    let tmp_dir = TempDir::new().unwrap();
    let src = tmp_dir.path().join("src");
    fs::create_dir(&src).unwrap();
    let dest = tmp_dir.path().join("dest");
    fs::create_dir(&dest).unwrap();

    for i in 0..8 {
        File::create(src.join(format!("{i}.txt")))
            .unwrap()
            .write_all(format!("file {i}").as_bytes())
            .unwrap();
    }

    Command::new(cargo_bin!("cp2"))
        .arg("-p")
        .arg("2")
        .args((0..8).map(|i| src.join(format!("{i}.txt"))))
        .arg(&dest)
        .assert()
        .success();

    for i in 0..8 {
        assert_eq!(
            fs::read_to_string(dest.join(format!("{i}.txt"))).unwrap(),
            format!("file {i}")
        );
    }
}

#[test]
fn test_copy_file_to_same_location_fails() {
    let tmp_dir = TempDir::new().unwrap();
    let file = tmp_dir.path().join("file.txt");
    File::create(&file).unwrap().write_all(b"data").unwrap();

    // Copy file.txt into the directory where it already lives
    Command::new(cargo_bin!("cp2"))
        .arg(&file)
        .arg(tmp_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("same file"));
}

#[test]
fn test_copy_dir_into_itself_fails() {
    let tmp_dir = TempDir::new().unwrap();
    let source = create_test_src(&tmp_dir, &[("f.txt", b"data")]);

    // Try to copy source/ into source/ (dest is parent of source)
    Command::new(cargo_bin!("cp2"))
        .arg("-r")
        .arg(&source)
        .arg(&source)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot copy a directory into itself").or(
            predicate::str::contains("same directory"),
        ));
}
