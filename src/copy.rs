use indicatif::ProgressBar;
use std::path::Path;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BUFFER_SIZE: usize = 8 * 1024 * 1024; // 8MB chunks

// Copy a file with dual progress bars (file + main)
pub async fn copy_file_with_dual_progress(
    from: &Path,
    to: &Path,
    file_pb: Option<&ProgressBar>,
    main_pb: Option<&ProgressBar>,
    sync: bool,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut source = fs::File::open(from).await?;
    let mut dest = fs::File::create(to).await?;

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

    if sync {
        dest.flush().await?;
        dest.sync_all().await?;
    }

    Ok(total_bytes)
}
