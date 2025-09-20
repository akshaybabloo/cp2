use indicatif::ProgressBar;
use std::path::{Component, Path, PathBuf};
use tokio::fs;

// Helper function to normalize a path, resolving `.` and `..` components.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::RootDir) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Normal(c) => ret.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            _ => {
                // Ignore RootDir and Prefix on non-Windows systems
            }
        }
    }
    ret
}

pub async fn copy_dir_recursive(
    from: &Path,
    to: &Path,
    pb: Option<&ProgressBar>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let from_normalized = normalize_path(&cwd.join(from));
    let to_normalized = normalize_path(&cwd.join(to));

    if to_normalized.starts_with(&from_normalized) && to_normalized != from_normalized {
        return Err("cannot copy a directory into itself".into());
    }

    // Create the destination directory if it doesn't exist
    fs::create_dir_all(to).await?;

    let mut entries = fs::read_dir(from).await?;

    while let Some(entry) = entries.next_entry().await? {
        let entry_path = entry.path();
        let relative_path = entry_path.strip_prefix(from)?;
        let dest_path = to.join(relative_path);

        if entry.file_type().await?.is_dir() {
            // Recursively copy subdirectories
            Box::pin(copy_dir_recursive(&entry_path, &dest_path, pb)).await?;
        } else {
            // Copy files
            let file_size = if pb.is_some() {
                fs::metadata(&entry_path).await?.len()
            } else {
                0
            };
            fs::copy(&entry_path, &dest_path).await?;
            if let Some(pb) = pb {
                pb.inc(file_size);
            }
        }
    }
    Ok(())
}
