use crate::config;
use crate::s3::{self, S3UploadEntry};
use crate::utils::trim_filename;
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// Runs an S3 upload for the given sources.
pub(crate) async fn run(
    sources: Vec<String>,
    remote_name: String,
    bucket: String,
    prefix: String,
    recursive: bool,
    parallel: usize,
    is_quiet: bool,
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
    let mut has_errors = false;
    let mut seen_keys: HashSet<String> = HashSet::new();

    for source_str in &sources {
        let source = Path::new(source_str);
        if !source.exists() {
            eprintln!(
                "{} {}",
                "Source path does not exist:".red(),
                source_str.red()
            );
            has_errors = true;
            continue;
        }
        if source.is_dir() && !recursive {
            eprintln!(
                "{} {}",
                "Source path is a directory, but recursive flag is not set:".red(),
                source_str.red()
            );
            has_errors = true;
            continue;
        }

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

    let (multi_progress, main_pb) = if !is_quiet {
        let multi = MultiProgress::new();
        let main_pb = multi.add(ProgressBar::new(total_size));
        main_pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("=>-"),
        );
        main_pb.set_message("Overall progress");
        main_pb.enable_steady_tick(std::time::Duration::from_millis(100));
        (Some(Arc::new(multi)), Some(Arc::new(main_pb)))
    } else {
        (None, None)
    };

    let semaphore = Arc::new(Semaphore::new(parallel));
    let has_failed = Arc::new(Mutex::new(has_errors));
    let mut tasks = Vec::new();

    for entry in all_entries {
        let sem = Arc::clone(&semaphore);
        let client_clone = Arc::clone(&client);
        let bucket_clone = bucket.clone();
        let multi_clone = multi_progress.as_ref().map(Arc::clone);
        let main_pb_clone = main_pb.as_ref().map(Arc::clone);
        let has_failed_clone = Arc::clone(&has_failed);

        tasks.push(tokio::spawn(async move {
            let _permit = sem
                .acquire()
                .await
                .expect("failed to acquire semaphore permit");

            let file_pb = if let Some(ref multi) = multi_clone {
                let file_name = entry
                    .from
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let pb = multi.add(ProgressBar::new(entry.size));
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template(
                            "  {spinner:.green} {msg:<30} [{wide_bar:.yellow/blue}] {bytes}/{total_bytes}",
                        )
                        .unwrap()
                        .progress_chars("=>-"),
                );
                let display_name = trim_filename(file_name, 28);
                pb.set_message(format!("Uploading {}", display_name));
                Some(pb)
            } else {
                None
            };

            match s3::upload_file(
                &client_clone,
                &entry.from,
                &bucket_clone,
                &entry.key,
                file_pb.as_ref(),
                main_pb_clone.as_deref(),
            )
            .await
            {
                Ok(_) => {
                    if let Some(ref pb) = file_pb {
                        pb.finish_and_clear();
                    }
                }
                Err(e) => {
                    if let Some(ref pb) = file_pb {
                        pb.finish_and_clear();
                    }
                    eprintln!("Error uploading file: {}", e);
                    *has_failed_clone.lock().unwrap() = true;
                }
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            eprintln!("Upload task failed: {}", e);
            *has_failed.lock().unwrap() = true;
        }
    }

    if let Some(pb) = main_pb {
        pb.finish_with_message("Upload complete!");
    }

    if *has_failed.lock().unwrap() {
        std::process::exit(1);
    }
}
