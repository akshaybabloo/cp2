use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Trims long file names for display
pub fn trim_filename(name: &str, max_len: usize) -> String {
    let char_count = name.chars().count();
    if char_count <= max_len {
        return name.to_string();
    }

    // If the name is too long, show start and end with ellipsis in the middle
    let ellipsis = "...";
    let ellipsis_len = ellipsis.len();

    if max_len <= ellipsis_len {
        return ellipsis.to_string();
    }

    let remaining = max_len - ellipsis_len;
    let start_len = remaining.div_ceil(2);
    let end_len = remaining / 2;

    let start: String = name.chars().take(start_len).collect();
    let end: String = name.chars().skip(char_count - end_len).collect();
    format!("{}{}{}", start, ellipsis, end)
}

/// Returns the final path component of `source`, resolving paths such as `.`
/// or `..` that have no file name of their own.
pub(crate) async fn source_name(source: &Path) -> Result<OsString, Box<dyn std::error::Error>> {
    if let Some(name) = source.file_name() {
        return Ok(name.to_owned());
    }
    fs::canonicalize(source)
        .await?
        .file_name()
        .map(|n| n.to_owned())
        .ok_or_else(|| format!("source has no file name: {}", source.display()).into())
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

    // Follow a symlink given as the source itself; symlinks found while
    // walking a directory are still skipped.
    let source_meta = fs::metadata(source).await?;

    if source_meta.file_type().is_file() {
        let file_name = source_name(source).await?;
        let dest = dest_base.join(file_name);

        // Reject same-file copies to avoid truncating the source
        if let (Ok(src_canon), Ok(dst_canon)) = (fs::canonicalize(source).await, fs::canonicalize(&dest).await)
            && src_canon == dst_canon
        {
            return Err(format!("source and destination are the same file: {}", source.display()).into());
        }

        let size = source_meta.len();
        entries.push(CopyEntry {
            from: source.to_path_buf(),
            to: dest,
            size,
        });
        return Ok((entries, dirs, 1, size));
    }

    if source_meta.file_type().is_dir() {
        let dir_name = source_name(source).await?;
        let dest_dir = dest_base.join(&dir_name);

        // Check for copy-into-self using canonicalized paths.
        // dest_dir may not exist yet, so canonicalize source and dest_base
        // separately, then append dir_name to the canonical dest_base.
        let src_canon = fs::canonicalize(source).await?;
        let dest_base_canon = fs::canonicalize(dest_base).await?;
        let dest_canon = dest_base_canon.join(&dir_name);
        if dest_canon.starts_with(&src_canon) {
            return Err(if dest_canon == src_canon {
                "source and destination are the same directory"
            } else {
                "cannot copy a directory into itself"
            }
            .into());
        }

        dirs.push(dest_dir.clone());

        let mut stack = vec![source.to_path_buf()];
        while let Some(p) = stack.pop() {
            let meta = if p == source {
                fs::metadata(&p).await?
            } else {
                fs::symlink_metadata(&p).await?
            };
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
                entries.push(CopyEntry {
                    from: p,
                    to: dest,
                    size,
                });
            }
            // Symlinks and other special file types are skipped
        }
    } else {
        return Err(format!("source is not a regular file or directory: {}", source.display()).into());
    }

    Ok((entries, dirs, total_count, total_size))
}
