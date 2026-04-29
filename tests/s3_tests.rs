/// Tests for S3-specific helpers exposed through the public modules.
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use tempfile::TempDir;

// ─── config module tests ──────────────────────────────────────────────────────

#[test]
fn test_config_save_and_load_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let mut cfg = HashMap::new();
    cfg.insert(
        "myremote".to_string(),
        cp2::config::RemoteConfig {
            remote_type: "s3".to_string(),
            provider: Some("AWS".to_string()),
            access_key_id: Some("AKID".to_string()),
            secret_access_key: Some("SECRET".to_string()),
            region: Some("us-east-1".to_string()),
            endpoint: None,
        },
    );

    // Write manually to a custom path to avoid touching the real config.
    let content = toml::to_string_pretty(&cfg).unwrap();
    fs::write(&config_path, content).unwrap();

    let loaded: cp2::config::Config = toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();

    let r = loaded.get("myremote").expect("remote should exist");
    assert_eq!(r.remote_type, "s3");
    assert_eq!(r.provider.as_deref(), Some("AWS"));
    assert_eq!(r.access_key_id.as_deref(), Some("AKID"));
    assert_eq!(r.secret_access_key.as_deref(), Some("SECRET"));
    assert_eq!(r.region.as_deref(), Some("us-east-1"));
    assert!(r.endpoint.is_none());
}

#[test]
fn test_config_with_endpoint() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let mut cfg = HashMap::new();
    cfg.insert(
        "minio".to_string(),
        cp2::config::RemoteConfig {
            remote_type: "s3".to_string(),
            provider: Some("Minio".to_string()),
            access_key_id: Some("minio".to_string()),
            secret_access_key: Some("miniosecret".to_string()),
            region: Some("us-east-1".to_string()),
            endpoint: Some("http://localhost:9000".to_string()),
        },
    );

    let content = toml::to_string_pretty(&cfg).unwrap();
    fs::write(&config_path, content).unwrap();

    let loaded: cp2::config::Config = toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    let r = loaded.get("minio").expect("remote should exist");
    assert_eq!(r.endpoint.as_deref(), Some("http://localhost:9000"));
}

#[test]
fn test_config_get_remote_present() {
    let mut cfg = cp2::config::Config::new();
    cfg.insert(
        "aws".to_string(),
        cp2::config::RemoteConfig {
            remote_type: "s3".to_string(),
            provider: None,
            access_key_id: None,
            secret_access_key: None,
            region: None,
            endpoint: None,
        },
    );
    assert!(cp2::config::get_remote(&cfg, "aws").is_some());
    assert!(cp2::config::get_remote(&cfg, "nonexistent").is_none());
}

// ─── collect_s3_upload_entries tests ─────────────────────────────────────────

#[tokio::test]
async fn test_s3_entries_single_file_no_prefix() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("data.txt");
    File::create(&file).unwrap().write_all(b"hello").unwrap();

    let (entries, count, size) = cp2::s3::collect_s3_upload_entries(&file, "").await.unwrap();

    assert_eq!(count, 1);
    assert_eq!(size, 5);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key, "data.txt");
}

#[tokio::test]
async fn test_s3_entries_single_file_with_prefix() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("report.csv");
    File::create(&file).unwrap().write_all(b"a,b").unwrap();

    let (entries, _count, _size) =
        cp2::s3::collect_s3_upload_entries(&file, "uploads/2024").await.unwrap();

    assert_eq!(entries[0].key, "uploads/2024/report.csv");
}

#[tokio::test]
async fn test_s3_entries_single_file_prefix_trailing_slash() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("img.png");
    File::create(&file).unwrap().write_all(b"data").unwrap();

    // Trailing slash on prefix should be normalised.
    let (entries, _count, _size) =
        cp2::s3::collect_s3_upload_entries(&file, "media/images/").await.unwrap();

    assert_eq!(entries[0].key, "media/images/img.png");
}

#[tokio::test]
async fn test_s3_entries_directory() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("mydir");
    fs::create_dir(&src).unwrap();
    File::create(src.join("a.txt")).unwrap().write_all(b"1").unwrap();
    fs::create_dir(src.join("sub")).unwrap();
    File::create(src.join("sub").join("b.txt")).unwrap().write_all(b"22").unwrap();

    let (entries, count, size) =
        cp2::s3::collect_s3_upload_entries(&src, "backup").await.unwrap();

    assert_eq!(count, 2);
    assert_eq!(size, 3);

    let keys: std::collections::HashSet<String> = entries.into_iter().map(|e| e.key).collect();
    assert!(keys.contains("backup/mydir/a.txt"));
    assert!(keys.contains("backup/mydir/sub/b.txt"));
}

#[tokio::test]
async fn test_s3_entries_directory_no_prefix() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("assets");
    fs::create_dir(&src).unwrap();
    File::create(src.join("logo.svg")).unwrap().write_all(b"svg").unwrap();

    let (entries, _count, _size) =
        cp2::s3::collect_s3_upload_entries(&src, "").await.unwrap();

    assert_eq!(entries[0].key, "assets/logo.svg");
}
