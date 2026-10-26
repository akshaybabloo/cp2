use crate::config;
use crate::s3::{self, S3UploadEntry};
use crate::transfer::{self, Progress};
use colored::Colorize;
use indicatif::MultiProgress;
use std::collections::HashSet;
use std::sync::Arc;

/// Runs an S3 upload for the given sources.
pub(crate) async fn run(
    sources: Vec<String>,
    remote_name: String,
    bucket: String,
    prefix: String,
    recursive: bool,
    parallel: usize,
    multi: Option<MultiProgress>,
) {
    // Load and look up the remote config.
    let cfg = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {}", "Failed to load configuration:".red(), e);
            std::process::exit(1);
        }
    };

    let remote = match config::get_remote(&cfg, &remote_name) {
        Some(r) => r.clone(),
        None => {
            eprintln!(
                "{} \"{}\"{}",
                "Remote".red(),
                remote_name.red(),
                " not found. Run `cp2 config create <name>` to add it.".red()
            );
            std::process::exit(1);
        }
    };

    // Build S3 client.
    let client = match s3::create_client(&remote).await {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("{} {}", "Failed to create S3 client:".red(), e);
            std::process::exit(1);
        }
    };

    // Validate sources and collect upload entries.
    let mut all_entries: Vec<S3UploadEntry> = Vec::new();
    let mut total_size: u64 = 0;
    let mut seen_keys: HashSet<String> = HashSet::new();
    let (valid_sources, mut has_errors) = transfer::validate_sources(&sources, recursive);

    for source in &valid_sources {
        match s3::collect_s3_upload_entries(source, &prefix).await {
            Ok((entries, _count, _size)) => {
                for entry in entries {
                    if !seen_keys.insert(entry.key.clone()) {
                        eprintln!(
                            "{} {} -> {}",
                            "Duplicate destination key:".red(),
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
        if has_errors {
            std::process::exit(1);
        }
        eprintln!("Nothing to upload.");
        std::process::exit(1);
    }

    log::info!(
        "Total files to upload: {}, total size: {}",
        all_entries.len(),
        total_size
    );

    let progress = Progress::new(multi, total_size);
    let failed = transfer::run_parallel(
        all_entries,
        parallel,
        progress.as_ref(),
        "Uploading",
        move |entry: S3UploadEntry, file_pb, main_pb| {
            let client = Arc::clone(&client);
            let bucket = bucket.clone();
            async move {
                s3::upload_file(
                    &client,
                    &entry.from,
                    &bucket,
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

    if has_errors || failed {
        std::process::exit(1);
    }
}
