use crate::copy::copy_file_with_dual_progress;
use crate::utils::{collect_copy_entries, trim_filename, CopyEntry};
use clap::{CommandFactory, FromArgMatches, Parser};
use clap_verbosity_flag::Verbosity;
use colored::Colorize;
use std::collections::HashSet;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::Semaphore;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about=None)]
struct Args {
    /// Source files or directories
    #[arg(required = true)]
    source: Vec<String>,

    /// Destination directory
    #[arg(required = true)]
    destination: String,

    /// Enable recursive copying for directories
    #[arg(short, long, default_value_t = false)]
    recursive: bool,

    /// Interactive mode
    #[arg(short, long, default_value_t = false)]
    interactive: bool,

    /// parallel level (number of concurrent copy operations)
    #[arg(short, long, default_value_t = 4)]
    parallel: usize,

    #[command(flatten)]
    verbosity: Verbosity,
}

impl Args {
    fn command_with_dynamic_parallel() -> clap::Command {
        // Keep dynamic help (no dynamic default needed)
        let max = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        Args::command().mut_arg("parallel", move |arg| arg.help(format!("Parallel level (max: {max})")))
    }
}

pub async fn run() {
    let max = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    log::debug!("Max parallel level (number of CPU cores): {}", max);
    let matches = Args::command_with_dynamic_parallel().get_matches();
    let args = Args::from_arg_matches(&matches).expect("parse args");
    log::debug!("Parsed args: {:#?}", args);

    let parallel = args.parallel.min(max);
    log::debug!("Using parallel level: {}", parallel);

    let is_quiet = args.verbosity.is_silent();

    env_logger::Builder::new()
        .filter_level(args.verbosity.into())
        .init();

    let destination = Path::new(&args.destination);
    if !destination.exists() {
        log::debug!("Destination path does not exist: {}", args.destination);
        println!(
            "{} {}",
            "Destination path does not exist: ".red(),
            args.destination.red()
        );
        std::process::exit(1);
    }
    if !destination.is_dir() {
        log::debug!("Destination path is not a directory: {}", args.destination);
        println!(
            "{} {}",
            "Destination path is not a directory: ".red(),
            args.destination.red()
        );
        std::process::exit(1);
    }

    // Validate all sources upfront before showing progress bars
    let mut valid_sources = Vec::new();
    let mut has_errors = false;

    for source_str in &args.source {
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
        if source.is_dir() && !args.recursive {
            eprintln!(
                "{} {}",
                "Source path is a directory, but recursive flag is not set:".red(),
                source_str.red()
            );
            has_errors = true;
            continue;
        }
        valid_sources.push(source_str.clone());
    }

    // Exit if no valid sources remain
    if valid_sources.is_empty() {
        std::process::exit(1);
    }

    // Collect all files to copy in a single tree walk per source
    let mut all_entries: Vec<CopyEntry> = Vec::new();
    let mut all_dirs: Vec<std::path::PathBuf> = Vec::new();
    let mut total_size: u64 = 0;
    let mut dest_paths: HashSet<std::path::PathBuf> = HashSet::new();

    for source_str in &valid_sources {
        let source = Path::new(source_str);
        match collect_copy_entries(source, destination).await {
            Ok((entries, dirs, _count, size)) => {
                // Check for duplicate destination paths before extending
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

    log::info!(
        "Total files to copy: {}, total size: {}",
        all_entries.len(),
        total_size
    );

    // Create all destination directories upfront
    for dir in &all_dirs {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            eprintln!("{} {}", "Error creating directory:".red(), e.to_string().red());
            std::process::exit(1);
        }
    }

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
        let multi_clone = multi_progress.as_ref().map(Arc::clone);
        let main_pb_clone = main_pb.as_ref().map(Arc::clone);
        let has_failed_clone = Arc::clone(&has_failed);

        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("failed to acquire semaphore permit");

            // Create per-file progress bar (no steady tick to reduce overhead)
            let file_pb = if let Some(ref multi) = multi_clone {
                let file_name = entry.from.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                let pb = multi.add(ProgressBar::new(entry.size));
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("  {spinner:.green} {msg:<30} [{wide_bar:.yellow/blue}] {bytes}/{total_bytes}")
                        .unwrap()
                        .progress_chars("=>-"),
                );
                let display_name = trim_filename(file_name, 28);
                pb.set_message(format!("Copying {}", display_name));
                Some(pb)
            } else {
                None
            };

            match copy_file_with_dual_progress(
                &entry.from,
                &entry.to,
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
                    eprintln!("Error copying file: {}", e);
                    *has_failed_clone.lock().unwrap() = true;
                }
            }
        }));
    }

    for task in tasks {
        task.await.expect("copy task failed");
    }

    if let Some(pb) = main_pb {
        pb.finish_with_message("Copy complete!");
    }

    if *has_failed.lock().unwrap() {
        std::process::exit(1);
    }
}
