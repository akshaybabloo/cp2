use cp2::copy::copy_dir_recursive;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
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

    for path in paths1 {
        let path1_full = dir1.join(&path);
        let path2_full = dir2.join(&path);
        if path1_full.is_file() {
            let mut content1 = Vec::new();
            File::open(path1_full).unwrap().read_to_end(&mut content1).unwrap();
            let mut content2 = Vec::new();
            File::open(path2_full).unwrap().read_to_end(&mut content2).unwrap();
            assert_eq!(content1, content2, "File contents are not equal for {:?}", path);
        }
    }
}

#[tokio::test]
async fn test_copy_simple_directory() {
    let tmp_dir = TempDir::new().unwrap();
    let structure: &[(&str, &[u8])] = &[("file1.txt", b"hello"), ("file2.txt", b"world")];
    let source = create_test_src(&tmp_dir, structure);
    let dest = tmp_dir.path().join("dest");

    copy_dir_recursive(&source, &dest, None).await.unwrap();

    assert_dirs_equal(&source, &dest);
}

#[tokio::test]
async fn test_copy_nested_directory() {
    let tmp_dir = TempDir::new().unwrap();
    let structure: &[(&str, &[u8])] = &[
        ("file1.txt", b"root file"),
        ("subdir/", b""),
        ("subdir/file2.txt", b"nested file"),
        ("subdir/another/", b""),
        ("subdir/another/file3.txt", b"deeply nested"),
    ];
    let source = create_test_src(&tmp_dir, structure);
    let dest = tmp_dir.path().join("dest");

    copy_dir_recursive(&source, &dest, None).await.unwrap();

    assert_dirs_equal(&source, &dest);
}

#[tokio::test]
async fn test_copy_empty_directory() {
    let tmp_dir = TempDir::new().unwrap();
    let structure = &[];
    let source = create_test_src(&tmp_dir, structure);
    let dest = tmp_dir.path().join("dest");

    copy_dir_recursive(&source, &dest, None).await.unwrap();

    assert!(dest.exists());
    assert!(fs::read_dir(&dest).unwrap().next().is_none());
}

#[tokio::test]
async fn test_copy_into_self_fails() {
    let tmp_dir = TempDir::new().unwrap();
    let source = create_test_src(&tmp_dir, &[]);
    let dest = source.join("sub"); // dest is inside source

    let result = copy_dir_recursive(&source, &dest, None).await;
    assert!(result.is_err());
    assert_eq!(result.err().unwrap().to_string(), "cannot copy a directory into itself");
}

#[tokio::test]
async fn test_copy_into_deep_self_fails() {
    let tmp_dir = TempDir::new().unwrap();
    let source = create_test_src(&tmp_dir, &[]);
    // Destination is deep inside the source directory
    let dest = source.join("sub").join("deeper").join("deepest");

    let result = copy_dir_recursive(&source, &dest, None).await;
    assert!(result.is_err());
    assert_eq!(result.err().unwrap().to_string(), "cannot copy a directory into itself");
}
