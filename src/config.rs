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

/// Returns the path to the cp2 configuration file.
///
/// Defaults to `~/.config/cp2/config.toml` following the XDG Base Directory
/// Specification on Linux/macOS.
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cp2")
        .join("config.toml")
}

/// Loads the configuration file from disk.  Returns an empty map if the file
/// does not exist yet.
pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let path = config_path();
    if !path.exists() {
        return Ok(Config::new());
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&content)?)
}

/// Persists the configuration map to disk, creating the directory if needed.
pub fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, toml::to_string_pretty(config)?)?;
    Ok(())
}

/// Looks up a named remote in the loaded configuration.
pub fn get_remote<'a>(config: &'a Config, name: &str) -> Option<&'a RemoteConfig> {
    config.get(name)
}
