use std::path::{Path, PathBuf};
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

        // Reject same-file copies to avoid truncating the source
        if let (Ok(src_canon), Ok(dst_canon)) =
            (fs::canonicalize(source).await, fs::canonicalize(&dest).await)
        {
            if src_canon == dst_canon {
                return Err(format!(
                    "source and destination are the same file: {}",
                    source.display()
                )
                .into());
            }
        }

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

        // Check for copy-into-self using canonicalized paths.
        // dest_dir may not exist yet, so canonicalize source and dest_base
        // separately, then append dir_name to the canonical dest_base.
        let src_canon = fs::canonicalize(source).await?;
        let dest_base_canon = fs::canonicalize(dest_base).await?;
        let dest_canon = dest_base_canon.join(dir_name);
        if dest_canon.starts_with(&src_canon) && dest_canon != src_canon {
            return Err("cannot copy a directory into itself".into());
        }

        dirs.push(dest_dir.clone());

        let mut stack = vec![source.to_path_buf()];
        while let Some(p) = stack.pop() {
            let meta = fs::symlink_metadata(&p).await?;
            if meta.file_type().is_dir() {
                if p != source {
                    let relative = p.strip_prefix(source)?;
                    dirs.push(dest_dir.join(relative));
                }
                let mut dir_entries = fs::read_dir(&p).await?;
                while let Some(entry) = dir_entries.next_entry().await? {
                    stack.push(entry.path());
                }
            } else if meta.file_type().is_file() {
                let relative = p.strip_prefix(source)?;
                let dest = dest_dir.join(relative);
                let size = meta.len();
                total_count += 1;
                total_size += size;
                entries.push(CopyEntry { from: p, to: dest, size });
            }
            // Symlinks and other special file types are skipped
        }
    }

    Ok((entries, dirs, total_count, total_size))
}
