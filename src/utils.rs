use std::path::{Path, PathBuf};
use tokio::fs;

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
