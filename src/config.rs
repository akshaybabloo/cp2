use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for an S3-compatible remote.
///
/// Intentionally does not derive `Debug`: the struct holds plaintext
/// credentials and we don't want them landing in log lines or panic
/// messages.
#[derive(Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// Remote type – currently only "s3" is supported.
    #[serde(rename = "type")]
    pub remote_type: String,
    /// Provider hint (e.g. "AWS", "Minio", "DigitalOcean").
    pub provider: Option<String>,
    /// AWS / compatible access key ID.
    pub access_key_id: Option<String>,
    /// AWS / compatible secret access key.
    pub secret_access_key: Option<String>,
    /// AWS region (e.g. "us-east-1").
    pub region: Option<String>,
    /// Custom endpoint URL for S3-compatible services (leave empty for AWS).
    pub endpoint: Option<String>,
}

/// In-memory representation of the whole config file.
pub type Config = HashMap<String, RemoteConfig>;

/// Environment variable that overrides the default config path. Primarily
/// used by tests so they don't clobber the user's real configuration.
const CONFIG_PATH_ENV: &str = "CP2_CONFIG";

/// Returns the path to the cp2 configuration file.
///
/// Honors the `CP2_CONFIG` environment variable when set; otherwise falls
/// back to `$XDG_CONFIG_HOME/cp2/config.toml` (typically
/// `~/.config/cp2/config.toml` on Linux/macOS).
pub fn config_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os(CONFIG_PATH_ENV) {
        return Ok(PathBuf::from(path));
    }
    let dir = dirs::config_dir().ok_or("could not determine the user config directory; set CP2_CONFIG to override")?;
    Ok(dir.join("cp2").join("config.toml"))
}

/// Loads the configuration file from disk.  Returns an empty map if the file
/// does not exist yet.
pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::new());
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&content)?)
}

/// Persists the configuration map to disk, creating the directory if needed.
/// On unix, the file is written atomically with mode 0600 so the plaintext
/// credentials are never readable by other users and a failed save leaves the
/// previous config intact.
pub fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        create_secure_dir(parent)?;
    }
    let serialized = toml::to_string_pretty(config)?;
    write_secure(&path, serialized.as_bytes())?;
    Ok(())
}

/// Looks up a named remote in the loaded configuration.
pub fn get_remote<'a>(config: &'a Config, name: &str) -> Option<&'a RemoteConfig> {
    config.get(name)
}

/// Writes `contents` to a new 0600 file next to `path`, then renames it over
/// `path`. The existing config is untouched unless the whole write succeeds.
#[cfg(unix)]
fn write_secure(path: &std::path::Path, contents: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    // Write through a symlinked config (e.g. one managed in a dotfiles repo)
    // rather than replacing the link.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };

    // In a world-writable directory without the sticky bit, another user
    // could replace the config after it is written.
    let mode = std::fs::metadata(parent)?.permissions().mode();
    if mode & 0o002 != 0 && mode & 0o1000 == 0 {
        return Err(format!(
            "refusing to write the config: {} is writable by all users",
            parent.display()
        )
        .into());
    }

    let file_name = path
        .file_name()
        .ok_or("config path has no file name")?
        .to_string_lossy();
    let tmp = parent.join(format!(".{}.{}.tmp", file_name, std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

#[cfg(not(unix))]
fn write_secure(path: &std::path::Path, contents: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(path, contents)?;
    Ok(())
}

#[cfg(unix)]
fn create_secure_dir(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::DirBuilderExt;
    // An existing directory is left alone: CP2_CONFIG may point into a shared
    // directory (e.g. $HOME or /tmp), and the file itself is always 0600.
    if !path.exists() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_secure_dir(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(path)?;
    Ok(())
}
