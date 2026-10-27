use crate::s3::{self, S3UploadEntry};
use crate::sftp::{self, SftpConnection};
use crate::transfer::{self, Progress};
use colored::Colorize;
use indicatif::MultiProgress;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Runs an SFTP upload of the given sources to `path` on `host`.
pub(crate) async fn run(
    sources: Vec<String>,
    host: String,
    path: String,
    recursive: bool,
    parallel: usize,
    multi: Option<MultiProgress>,
) {
    let (valid_sources, mut has_errors) = transfer::validate_sources(&sources, recursive);
    if valid_sources.is_empty() {
        std::process::exit(1);
    }

    // Connect once first, so any password or host-key prompt from `ssh`
    // happens on its own before the other connections start.
    let first = connect_or_exit(&host).await;
    let base = match sftp::resolve_base_dir(&first.session, &path).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} {}", "Error:".red(), e.to_string().red());
            std::process::exit(1);
        }
    };

    // Remote paths are built the same way as S3 keys: `<base>/<name>[/...]`.
    let mut all_entries: Vec<S3UploadEntry> = Vec::new();
    let mut total_size: u64 = 0;
    let mut seen_paths: HashSet<String> = HashSet::new();
    for source in &valid_sources {
        match s3::collect_s3_upload_entries(source, &base).await {
            Ok((entries, _count, _size)) => {
                for entry in entries {
                    if !seen_paths.insert(entry.key.clone()) {
                        eprintln!(
                            "{} {} -> {}",
                            "Duplicate destination path:".red(),
                            entry.from.display().to_string().red(),
                            entry.key.red(),
                        );
                        has_errors = true;
                        continue;
                    }
                    total_size += entry.size;
                    all_entries.push(entry);
                }
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red(), e.to_string().red());
                has_errors = true;
            }
        }
    }

    if all_entries.is_empty() {
        std::process::exit(1);
    }

    log::info!(
        "Total files to upload: {}, total size: {}",
        all_entries.len(),
        total_size
    );

    for dir in dirs_to_create(&base, &all_entries) {
        if let Err(e) = sftp::ensure_dir(&first.session, &dir).await {
            eprintln!("{} {}", "Error:".red(), e.to_string().red());
            std::process::exit(1);
        }
    }

    // One `ssh` process per parallel slot: a single SSH connection is often
    // limited by its window size on high-latency links.
    let mut connections = vec![first];
    for _ in 1..parallel.min(all_entries.len()) {
        connections.push(connect_or_exit(&host).await);
    }
    let sessions: Arc<Vec<_>> = Arc::new(connections.iter().map(|c| Arc::clone(&c.session)).collect());
    let next = Arc::new(AtomicUsize::new(0));

    let progress = Progress::new(multi, total_size);
    let failed = transfer::run_parallel(
        all_entries,
        parallel,
        progress.as_ref(),
        "Uploading",
        move |entry: S3UploadEntry, file_pb, main_pb| {
            let sessions = Arc::clone(&sessions);
            let i = next.fetch_add(1, Ordering::Relaxed) % sessions.len();
            async move {
                sftp::upload_file(
                    &sessions[i],
                    &entry.from,
                    &entry.key,
                    file_pb.as_ref(),
                    main_pb.as_ref(),
                )
                .await
                .map_err(|e| e.to_string())
            }
        },
    )
    .await;

    if let Some(progress) = progress {
        progress.finish("Upload complete!");
    }

    for connection in connections {
        connection.close().await;
    }

    if has_errors || failed {
        std::process::exit(1);
    }
}

async fn connect_or_exit(host: &str) -> SftpConnection {
    match SftpConnection::connect(host).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {}", "Error:".red(), e.to_string().red());
            std::process::exit(1);
        }
    }
}

/// Every directory between `base` and the files being uploaded, parents
/// before children.
fn dirs_to_create(base: &str, entries: &[S3UploadEntry]) -> BTreeSet<String> {
    let prefix = format!("{}/", base.trim_end_matches('/'));
    let mut dirs = BTreeSet::new();
    for entry in entries {
        let Some(relative) = entry.key.strip_prefix(&prefix) else {
            continue;
        };
        let mut dir = prefix.trim_end_matches('/').to_string();
        let components: Vec<&str> = relative.split('/').collect();
        for component in &components[..components.len() - 1] {
            dir = format!("{dir}/{component}");
            dirs.insert(dir.clone());
        }
    }
    dirs
}
