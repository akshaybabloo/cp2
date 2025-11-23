use indicatif::ProgressBar;
use std::path::{Component, Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BUFFER_SIZE: usize = 8 * 1024 * 1024; // 8MB chunks
const SYNC_INTERVAL: usize = 64 * 1024 * 1024; // Sync every 64MB

// Copy a file in chunks to allow progress updates
pub async fn copy_file_with_progress(
    from: &Path,
    to: &Path,
    pb: Option<&ProgressBar>,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut source = fs::File::open(from).await?;
    let mut dest = fs::File::create(to).await?;

    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut total_bytes = 0u64;
    let mut bytes_since_sync = 0usize;

    loop {
        let bytes_read = source.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }

        dest.write_all(&buffer[..bytes_read]).await?;
        bytes_since_sync += bytes_read;

        // Periodically sync to disk to ensure progress bar reflects actual writes
        if bytes_since_sync >= SYNC_INTERVAL {
            dest.sync_data().await?;
            bytes_since_sync = 0;
        }

        total_bytes += bytes_read as u64;

        if let Some(pb) = pb {
            pb.inc(bytes_read as u64);
        }
    }

    // Ensure all remaining data is flushed to the OS and synced to disk
    dest.flush().await?;
    dest.sync_all().await?;
    Ok(total_bytes)
}

// Copy a file with dual progress bars (file + main)
pub async fn copy_file_with_dual_progress(
    from: &Path,
    to: &Path,
    file_pb: Option<&ProgressBar>,
    main_pb: Option<&ProgressBar>,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut source = fs::File::open(from).await?;
    let mut dest = fs::File::create(to).await?;

    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut total_bytes = 0u64;
    let mut bytes_since_sync = 0usize;

    loop {
        let bytes_read = source.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }

        dest.write_all(&buffer[..bytes_read]).await?;
        bytes_since_sync += bytes_read;

        // Periodically sync to disk to ensure progress bar reflects actual writes
        if bytes_since_sync >= SYNC_INTERVAL {
            dest.sync_data().await?;
            bytes_since_sync = 0;
        }

        total_bytes += bytes_read as u64;

        if let Some(pb) = file_pb {
            pb.inc(bytes_read as u64);
        }
        if let Some(pb) = main_pb {
            pb.inc(bytes_read as u64);
        }
    }

    // Ensure all remaining data is flushed to the OS and synced to disk
    dest.flush().await?;
    dest.sync_all().await?;
    Ok(total_bytes)
}

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
            // Copy files with progress tracking
            copy_file_with_progress(&entry_path, &dest_path, pb).await?;
        }
    }
    Ok(())
}
