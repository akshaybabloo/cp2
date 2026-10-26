use indicatif::ProgressBar;
use std::fs::FileTimes;
use std::path::Path;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BUFFER_SIZE: usize = 8 * 1024 * 1024; // 8MB chunks

// Copy a file with dual progress bars (file + main). The destination keeps
// the source's permissions and access/modification times.
pub async fn copy_file_with_dual_progress(
    from: &Path,
    to: &Path,
    file_pb: Option<&ProgressBar>,
    main_pb: Option<&ProgressBar>,
    sync: bool,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut source = fs::File::open(from).await?;
    let source_meta = source.metadata().await?;
    let mut dest = create_dest(to).await?;

    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut total_bytes = 0u64;

    loop {
        let bytes_read = source.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }

        dest.write_all(&buffer[..bytes_read]).await?;
        total_bytes += bytes_read as u64;

        if let Some(pb) = file_pb {
            pb.inc(bytes_read as u64);
        }
        if let Some(pb) = main_pb {
            pb.inc(bytes_read as u64);
        }
    }

    // Always flush: tokio completes writes in the background, and an error
    // from the final write only surfaces here.
    dest.flush().await?;

    let dest = dest.into_std().await;
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        dest.set_permissions(source_meta.permissions())?;
        let mut times = FileTimes::new();
        if let Ok(modified) = source_meta.modified() {
            times = times.set_modified(modified);
        }
        if let Ok(accessed) = source_meta.accessed() {
            times = times.set_accessed(accessed);
        }
        dest.set_times(times)?;
        if sync {
            dest.sync_all()?;
        }
        Ok(())
    })
    .await??;

    Ok(total_bytes)
}

/// Creates (or truncates) `to` for writing. An existing read-only file, such
/// as one left by an earlier copy of a read-only source, is replaced.
async fn create_dest(to: &Path) -> std::io::Result<fs::File> {
    match fs::File::create(to).await {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && fs::symlink_metadata(to).await.is_ok() => {
            fs::remove_file(to).await?;
            fs::File::create(to).await
        }
        result => result,
    }
}
