use cp2::utils::get_copy_size;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

// Helper to create a directory structure for testing size calculation.
fn create_test_dir(tmp_dir: &TempDir, structure: &[(&str, &[u8])]) -> PathBuf {
    let root = tmp_dir.path().join("size_test_root");
    fs::create_dir_all(&root).unwrap();
    for (path, content) in structure {
        let full_path = root.join(path);
        if path.ends_with('/') {
            fs::create_dir_all(full_path).unwrap();
        } else {
            fs::create_dir_all(full_path.parent().unwrap()).unwrap();
            let mut file = File::create(full_path).unwrap();
            file.write_all(content).unwrap();
        }
    }
    root
}

#[tokio::test]
async fn test_get_size_single_file() {
    let tmp_dir = TempDir::new().unwrap();
    let root = create_test_dir(&tmp_dir, &[("file.txt", b"12345")]);
    let file_path = root.join("file.txt");

    let (count, size) = get_copy_size(&file_path).await;
    assert_eq!(count, 1);
    assert_eq!(size, 5);
}

#[tokio::test]
async fn test_get_size_empty_dir() {
    let tmp_dir = TempDir::new().unwrap();
    let root = create_test_dir(&tmp_dir, &[]);

    let (count, size) = get_copy_size(&root).await;
    assert_eq!(count, 0);
    assert_eq!(size, 0);
}

#[tokio::test]
async fn test_get_size_flat_dir() {
    let tmp_dir = TempDir::new().unwrap();
    let root = create_test_dir(&tmp_dir, &[("file1.txt", b"123"), ("file2.txt", b"4567")]);

    let (count, size) = get_copy_size(&root).await;
    assert_eq!(count, 2);
    assert_eq!(size, 7);
}

#[tokio::test]
async fn test_get_size_nested_dir() {
    let tmp_dir = TempDir::new().unwrap();
    let root = create_test_dir(
        &tmp_dir,
        &[
            ("file1.txt", b"1"),
            ("sub/", b""),
            ("sub/file2.txt", b"22"),
            ("sub/deep/file3.txt", b"333"),
        ],
    );

    let (count, size) = get_copy_size(&root).await;
    assert_eq!(count, 3);
    assert_eq!(size, 6);
}

#[tokio::test]
async fn test_get_size_non_existent_path() {
    let tmp_dir = TempDir::new().unwrap();
    let non_existent_path = tmp_dir.path().join("does_not_exist");

    let (count, size) = get_copy_size(&non_existent_path).await;
    assert_eq!(count, 0);
    assert_eq!(size, 0);
}
