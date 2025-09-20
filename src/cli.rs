use crate::copy::copy_dir_recursive;
use crate::utils::get_copy_size;
use clap::{CommandFactory, FromArgMatches, Parser};
use clap_verbosity_flag::Verbosity;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::{sync::Arc, thread};
use tokio::{fs, sync::Semaphore};

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

    /// Overwrite existing files without prompt
    #[arg(short, long, default_value_t = false)]
    force: bool,

    /// Interactive mode
    #[arg(short, long, default_value_t = false)]
    interactive: bool,

    /// parallel level (number of concurrent copy operations)
    #[arg(short, long, default_value_t = 4)]
    parallel: usize,

    #[command(flatten)]
    verbosity: Verbosity,

    /// Check copied files for integrity
    #[arg(short, long, default_value_t = false)]
    check: bool,
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

    let pb = if !is_quiet {
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
        let pb = ProgressBar::new(total_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} {msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})",
                )
                .unwrap()
                .progress_chars("=>-"),
        );
        pb.set_message("Copying...");
        Some(Arc::new(pb))
    } else {
        None
    };

    let semaphore = Arc::new(Semaphore::new(parallel));
    let mut tasks = Vec::new();

    for source_str in args.source {
        let destination = destination.to_path_buf();
        let recursive = args.recursive;
        let sem = Arc::clone(&semaphore);
        let pb_clone = pb.as_ref().map(Arc::clone);

        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("failed to acquire semaphore permit");
            let source = Path::new(&source_str);
            if !source.exists() {
                log::error!("Source path does not exist: {}", source_str);
                return;
            }
            if source.is_dir() && !recursive {
                log::error!(
                    "Source path is a directory, but recursive flag is not set: {}",
                    source_str
                );
                return;
            }

            if source.is_file() {
                let file_name = source.file_name().unwrap_or_else(|| std::ffi::OsStr::new(&source_str));
                let dest_path = destination.join(file_name);
                let file_size = if pb_clone.is_some() {
                    fs::metadata(source).await.map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                };
                match fs::copy(source, &dest_path).await {
                    Ok(_) => {
                        if let Some(pb) = pb_clone {
                            pb.inc(file_size);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error copying file: {}", e);
                    }
                }
            } else if source.is_dir() {
                let dir_name = source.file_name().unwrap_or_else(|| std::ffi::OsStr::new(&source_str));
                let dest_path = destination.join(dir_name);
                if let Err(e) = copy_dir_recursive(source, &dest_path, pb_clone.as_deref()).await {
                    eprintln!("Error copying directory: {}", e);
                }
            }
        }));
    }

    for task in tasks {
        task.await.expect("copy task failed");
    }

    if let Some(pb) = pb {
        pb.finish_with_message("Copy complete!");
    }
}
