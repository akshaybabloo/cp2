/// Tests for S3-specific helpers exposed through the public modules.
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::sync::Mutex;
use tempfile::TempDir;

// Serializes tests that mutate the process-wide CP2_CONFIG env var.
static CONFIG_ENV_LOCK: Mutex<()> = Mutex::new(());

struct ConfigEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl ConfigEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let lock = CONFIG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("CP2_CONFIG");
        // SAFETY: the static mutex serializes all CP2_CONFIG mutations within
        // this test binary, so no other thread is reading the env at this
        // moment.
        unsafe {
            std::env::set_var("CP2_CONFIG", path);
        }
        Self { _lock: lock, previous }
    }
}

impl Drop for ConfigEnvGuard {
    fn drop(&mut self) {
        // SAFETY: same reasoning as `set`.
        unsafe {
            match self.previous.as_ref() {
                Some(prev) => std::env::set_var("CP2_CONFIG", prev),
                None => std::env::remove_var("CP2_CONFIG"),
            }
        }
    }
}

// ─── config module tests ──────────────────────────────────────────────────────

#[test]
fn test_config_save_and_load_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let _guard = ConfigEnvGuard::set(&config_path);

    let mut cfg: cp2::config::Config = HashMap::new();
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

    cp2::config::save_config(&cfg).expect("save");
    let loaded = cp2::config::load_config().expect("load");

    let r = loaded.get("myremote").expect("remote should exist");
    assert_eq!(r.remote_type, "s3");
    assert_eq!(r.provider.as_deref(), Some("AWS"));
    assert_eq!(r.access_key_id.as_deref(), Some("AKID"));
    assert_eq!(r.secret_access_key.as_deref(), Some("SECRET"));
    assert_eq!(r.region.as_deref(), Some("us-east-1"));
    assert!(r.endpoint.is_none());
}

#[test]
fn test_config_load_returns_empty_when_missing() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("missing.toml");
    let _guard = ConfigEnvGuard::set(&config_path);

    let loaded = cp2::config::load_config().expect("load");
    assert!(loaded.is_empty());
}

#[test]
fn test_config_with_endpoint() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let _guard = ConfigEnvGuard::set(&config_path);

    let mut cfg: cp2::config::Config = HashMap::new();
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

    cp2::config::save_config(&cfg).expect("save");
    let loaded = cp2::config::load_config().expect("load");
    let r = loaded.get("minio").expect("remote should exist");
    assert_eq!(r.endpoint.as_deref(), Some("http://localhost:9000"));
}

#[cfg(unix)]
#[test]
fn test_config_file_is_chmod_600() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let _guard = ConfigEnvGuard::set(&config_path);

    let mut cfg: cp2::config::Config = HashMap::new();
    cfg.insert(
        "x".to_string(),
        cp2::config::RemoteConfig {
            remote_type: "s3".to_string(),
            provider: None,
            access_key_id: None,
            secret_access_key: None,
            region: None,
            endpoint: None,
        },
    );
    cp2::config::save_config(&cfg).expect("save");

    let mode = fs::metadata(&config_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "config file must be readable only by the owner");
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

// ─── pick_part_size tests ─────────────────────────────────────────────────────

const MIN_PART_SIZE: u64 = 8 * 1024 * 1024;
const MAX_PARTS: u64 = 10_000;

#[test]
fn test_part_size_small_file_uses_min() {
    assert_eq!(cp2::s3::pick_part_size(1024).unwrap(), MIN_PART_SIZE);
    assert_eq!(cp2::s3::pick_part_size(MIN_PART_SIZE).unwrap(), MIN_PART_SIZE);
}

#[test]
fn test_part_size_under_part_limit() {
    // 10 GiB < 10000 * 8 MiB, so default 8 MiB still works.
    let ten_gib = 10 * 1024 * 1024 * 1024_u64;
    assert_eq!(cp2::s3::pick_part_size(ten_gib).unwrap(), MIN_PART_SIZE);
}

#[test]
fn test_part_size_scales_for_huge_file() {
    // 1 TiB requires part size > 8 MiB to stay under 10000 parts.
    let one_tib = 1024 * 1024 * 1024 * 1024_u64;
    let ps = cp2::s3::pick_part_size(one_tib).unwrap();
    assert!(ps > MIN_PART_SIZE);
    assert!(one_tib.div_ceil(ps) <= MAX_PARTS);
}

#[test]
fn test_part_size_rejects_files_above_5_tib() {
    // 50 TiB forces a part size above S3's 5 GiB max.
    let way_too_big = 50 * 1024 * 1024 * 1024 * 1024_u64;
    assert!(cp2::s3::pick_part_size(way_too_big).is_err());
}
