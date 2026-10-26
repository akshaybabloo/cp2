use crate::copy::copy_file_with_dual_progress;
use crate::transfer::{self, Progress};
use crate::utils::{CopyEntry, collect_copy_entries};
use colored::Colorize;
use indicatif::MultiProgress;
use std::collections::HashSet;

/// Runs a local filesystem copy for the given sources.
pub(crate) async fn run(
    sources: Vec<String>,
    destination: std::path::PathBuf,
    recursive: bool,
    sync: bool,
    parallel: usize,
    multi: Option<MultiProgress>,
) {
    if !destination.exists() {
        log::debug!("Destination path does not exist: {}", destination.display());
        eprintln!(
            "{} {}",
            "Destination path does not exist: ".red(),
            destination.display().to_string().red()
        );
        std::process::exit(1);
    }
    if !destination.is_dir() {
        log::debug!("Destination path is not a directory: {}", destination.display());
        eprintln!(
            "{} {}",
            "Destination path is not a directory: ".red(),
            destination.display().to_string().red()
        );
        std::process::exit(1);
    }

    let (valid_sources, mut has_errors) = transfer::validate_sources(&sources, recursive);
    if valid_sources.is_empty() {
        std::process::exit(1);
    }

    // Collect all copy entries.
    let mut all_entries: Vec<CopyEntry> = Vec::new();
    let mut all_dirs: Vec<std::path::PathBuf> = Vec::new();
    let mut total_size: u64 = 0;
    let mut dest_paths: HashSet<std::path::PathBuf> = HashSet::new();

    for source in &valid_sources {
        match collect_copy_entries(source, &destination).await {
            Ok((entries, dirs, _count, size)) => {
                let mut source_has_dup = false;
                for entry in &entries {
                    if dest_paths.contains(&entry.to) {
                        eprintln!(
                            "{} {} -> {}",
                            "Duplicate destination path:".red(),
                            entry.from.display().to_string().red(),
                            entry.to.display().to_string().red()
                        );
                        has_errors = true;
                        source_has_dup = true;
                    }
                }
                if source_has_dup {
                    continue;
                }
                for entry in &entries {
                    dest_paths.insert(entry.to.clone());
                }
                all_entries.extend(entries);
                all_dirs.extend(dirs);
                total_size += size;
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red(), e.to_string().red());
                has_errors = true;
            }
        }
    }

    if all_entries.is_empty() && all_dirs.is_empty() {
        std::process::exit(1);
    }

    log::info!("Total files to copy: {}, total size: {}", all_entries.len(), total_size);

    // Create destination directories upfront.
    for dir in &all_dirs {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            eprintln!("{} {}", "Error creating directory:".red(), e.to_string().red());
            std::process::exit(1);
        }
    }

    let progress = Progress::new(multi, total_size);
    let failed = transfer::run_parallel(
        all_entries,
        parallel,
        progress.as_ref(),
        "Copying",
        move |entry: CopyEntry, file_pb, main_pb| async move {
            copy_file_with_dual_progress(&entry.from, &entry.to, file_pb.as_ref(), main_pb.as_ref(), sync)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
    )
    .await;

    if let Some(progress) = progress {
        progress.finish("Copy complete!");
    }

    if has_errors || failed {
        std::process::exit(1);
    }
}
