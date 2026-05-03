use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for an S3-compatible remote.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let dir = dirs::config_dir().ok_or(
        "could not determine the user config directory; set CP2_CONFIG to override",
    )?;
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
/// On unix, restricts the file mode to 0600 since it contains plaintext
/// credentials.
pub fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, toml::to_string_pretty(config)?)?;
    set_secure_permissions(&path)?;
    Ok(())
}

/// Looks up a named remote in the loaded configuration.
pub fn get_remote<'a>(config: &'a Config, name: &str) -> Option<&'a RemoteConfig> {
    config.get(name)
}

#[cfg(unix)]
fn set_secure_permissions(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_secure_permissions(_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
