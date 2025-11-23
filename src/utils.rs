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

/// Recursively calculates the total number of files and their cumulative size in bytes
pub async fn get_copy_size(path: &Path) -> (u64, u64) {
    let mut num_files = 0;
    let mut total_size = 0;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];

    while let Some(p) = stack.pop() {
        if p.is_dir() {
            if let Ok(mut entries) = fs::read_dir(&p).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    stack.push(entry.path());
                }
            }
        } else if p.is_file() {
            num_files += 1;
            if let Ok(meta) = fs::metadata(&p).await {
                total_size += meta.len();
            }
        }
    }
    (num_files, total_size)
}
