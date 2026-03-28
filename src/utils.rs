use std::path::{Component, Path, PathBuf};
use tokio::fs;

/// Trims long file names for display
pub fn trim_filename(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        return name.to_string();
    }

    // If the name is too long, show start and end with ellipsis in the middle
    let ellipsis = "...";
    let ellipsis_len = ellipsis.len();

    if max_len <= ellipsis_len {
        return ellipsis.to_string();
    }

    let remaining = max_len - ellipsis_len;
    let start_len = (remaining + 1) / 2;
    let end_len = remaining / 2;

    format!("{}{}{}", &name[..start_len], ellipsis, &name[name.len() - end_len..])
}

/// Helper function to normalize a path, resolving `.` and `..` components.
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
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
            _ => {}
        }
    }
    ret
}

/// A file to be copied with source path, destination path, and size.
pub struct CopyEntry {
    pub from: PathBuf,
    pub to: PathBuf,
    pub size: u64,
}

/// Collects all files to copy from a source to a destination directory.
/// Walks the tree once, returning file entries, directories to create, total count, and total size.
pub async fn collect_copy_entries(
    source: &Path,
    dest_base: &Path,
) -> Result<(Vec<CopyEntry>, Vec<PathBuf>, u64, u64), Box<dyn std::error::Error>> {
    let mut entries = Vec::new();
    let mut dirs = Vec::new();
    let mut total_count = 0u64;
    let mut total_size = 0u64;

    if source.is_file() {
        let file_name = source.file_name().ok_or("source has no file name")?;
        let dest = dest_base.join(file_name);
        let size = fs::metadata(source).await.map(|m| m.len()).unwrap_or(0);
        entries.push(CopyEntry {
            from: source.to_path_buf(),
            to: dest,
            size,
        });
        return Ok((entries, dirs, 1, size));
    }

    if source.is_dir() {
        let dir_name = source.file_name().ok_or("source has no file name")?;
        let dest_dir = dest_base.join(dir_name);

        // Check for copy-into-self
        let cwd = std::env::current_dir()?;
        let src_norm = normalize_path(&cwd.join(source));
        let dest_norm = normalize_path(&cwd.join(&dest_dir));
        if dest_norm.starts_with(&src_norm) && dest_norm != src_norm {
            return Err("cannot copy a directory into itself".into());
        }

        dirs.push(dest_dir.clone());

        let mut stack = vec![source.to_path_buf()];
        while let Some(p) = stack.pop() {
            if p.is_dir() {
                if p != source {
                    let relative = p.strip_prefix(source)?;
                    dirs.push(dest_dir.join(relative));
                }
                if let Ok(mut dir_entries) = fs::read_dir(&p).await {
                    while let Ok(Some(entry)) = dir_entries.next_entry().await {
                        stack.push(entry.path());
                    }
                }
            } else if p.is_file() {
                let relative = p.strip_prefix(source)?;
                let dest = dest_dir.join(relative);
                let size = fs::metadata(&p).await.map(|m| m.len()).unwrap_or(0);
                total_count += 1;
                total_size += size;
                entries.push(CopyEntry { from: p, to: dest, size });
            }
        }
    }

    Ok((entries, dirs, total_count, total_size))
}
