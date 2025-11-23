use crate::copy::{copy_dir_recursive, copy_file_with_dual_progress};
use crate::utils::{get_copy_size, trim_filename};
use clap::{CommandFactory, FromArgMatches, Parser};
use clap_verbosity_flag::Verbosity;
use colored::Colorize;
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

    // /// Overwrite existing files without prompt
    // #[arg(short, long, default_value_t = false)]
    // force: bool,
    /// Interactive mode
    #[arg(short, long, default_value_t = false)]
    interactive: bool,

    /// parallel level (number of concurrent copy operations)
    #[arg(short, long, default_value_t = 4)]
    parallel: usize,

    #[command(flatten)]
    verbosity: Verbosity,
    // /// Check copied files for integrity
    // #[arg(short, long, default_value_t = false)]
    // check: bool,
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

    let is_quiet = args.verbosity.log_level().is_none();

    env_logger::Builder::new().filter_level(args.verbosity.into()).init();

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

    let (multi_progress, main_pb) = if !is_quiet {
        let mut total_files: u64 = 0;
        let mut total_size: u64 = 0;

        for source_str in &args.source {
            let source = Path::new(source_str);
            if !source.exists() {
                log::warn!("Source path does not exist: {}", source_str);
                continue;
            }
            if source.is_dir() && !args.recursive {
                log::warn!(
                    "Source path is a directory, but recursive flag is not set: {}",
                    source_str
                );
                continue;
            }
            let (files, size) = get_copy_size(source).await;
            total_files += files;
            total_size += size;
        }

        log::info!("Total files to copy: {}, total size: {}", total_files, total_size);

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
    let has_failed = Arc::new(Mutex::new(false));
    let mut tasks = Vec::new();

    for source_str in args.source {
        let destination = destination.to_path_buf();
        let recursive = args.recursive;
        let sem = Arc::clone(&semaphore);
        let multi_clone = multi_progress.as_ref().map(Arc::clone);
        let main_pb_clone = main_pb.as_ref().map(Arc::clone);
        let has_failed_clone = Arc::clone(&has_failed);

        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("failed to acquire semaphore permit");
            let source = Path::new(&source_str);
            if !source.exists() {
                log::error!("Source path does not exist: {}", source_str);
                *has_failed_clone.lock().unwrap() = true;
                return;
            }
            if source.is_dir() && !recursive {
                log::error!(
                    "Source path is a directory, but recursive flag is not set: {}",
                    source_str
                );
                *has_failed_clone.lock().unwrap() = true;
                return;
            }

            // Create individual progress bar for this file/directory
            let file_pb = if let Some(ref multi) = multi_clone {
                let file_name = source.file_name().and_then(|n| n.to_str()).unwrap_or(&source_str);

                let file_size = if source.is_file() {
                    tokio::fs::metadata(source).await.map(|m| m.len()).unwrap_or(0)
                } else {
                    0 // For directories, we'll update size as we go
                };

                let pb = multi.add(ProgressBar::new(file_size));
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("  {spinner:.green} {msg:<30} [{wide_bar:.yellow/blue}] {bytes}/{total_bytes}")
                        .unwrap()
                        .progress_chars("=>-"),
                );
                let display_name = trim_filename(file_name, 28);
                pb.set_message(format!("Copying {}", display_name));
                pb.enable_steady_tick(std::time::Duration::from_millis(100));
                Some(pb)
            } else {
                None
            };

            if source.is_file() {
                let file_name = source.file_name().unwrap_or_else(|| std::ffi::OsStr::new(&source_str));
                let dest_path = destination.join(file_name);

                match copy_file_with_dual_progress(source, &dest_path, file_pb.as_ref(), main_pb_clone.as_deref()).await
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
            } else if source.is_dir() {
                let dir_name = source.file_name().unwrap_or_else(|| std::ffi::OsStr::new(&source_str));
                let dest_path = destination.join(dir_name);

                match copy_dir_recursive(source, &dest_path, main_pb_clone.as_deref()).await {
                    Ok(_) => {
                        if let Some(ref pb) = file_pb {
                            pb.finish_and_clear();
                        }
                    }
                    Err(e) => {
                        if let Some(ref pb) = file_pb {
                            pb.finish_and_clear();
                        }
                        eprintln!("Error copying directory: {}", e);
                        *has_failed_clone.lock().unwrap() = true;
                    }
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
