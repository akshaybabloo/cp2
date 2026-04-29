use crate::config::{self, RemoteConfig};
use crate::copy::copy_file_with_dual_progress;
use crate::s3::{self, S3UploadEntry};
use crate::utils::{collect_copy_entries, trim_filename, CopyEntry};
use clap::{CommandFactory, FromArgMatches, Parser};
use clap_verbosity_flag::Verbosity;
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::Semaphore;

// ─── CLI definition ───────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Source files or directories
    #[arg(required = true)]
    source: Vec<String>,

    /// Destination directory (local path or remote:bucket/prefix)
    #[arg(required = true)]
    destination: String,

    /// Enable recursive copying for directories
    #[arg(short, long, default_value_t = false)]
    recursive: bool,

    /// Interactive mode
    #[arg(short, long, default_value_t = false)]
    interactive: bool,

    /// Parallel level (number of concurrent copy operations)
    #[arg(short, long, default_value_t = 4)]
    parallel: usize,

    /// Sync each file to disk after copying (slower, but crash-safe; local copies only)
    #[arg(short = 'S', long, default_value_t = false)]
    sync: bool,

    #[command(flatten)]
    verbosity: Verbosity,
}

impl Args {
    fn command_with_dynamic_parallel() -> clap::Command {
        let max = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        Args::command()
            .mut_arg("parallel", move |arg| arg.help(format!("Parallel level (max: {max})")))
    }
}

// ─── Destination type ─────────────────────────────────────────────────────────

enum Destination {
    /// A regular local filesystem path.
    Local(std::path::PathBuf),
    /// An S3 remote: `remote_name:bucket/prefix`.
    S3 {
        remote_name: String,
        bucket: String,
        /// Key prefix within the bucket (may be empty).
        prefix: String,
    },
}

/// Parses a destination string, distinguishing `remote:bucket/prefix` from a
/// plain local path.
fn parse_destination(dest: &str) -> Destination {
    if let Some(colon_pos) = dest.find(':') {
        let name = &dest[..colon_pos];
        // A remote name must be non-empty and must not contain path separators.
        if !name.is_empty() && !name.contains('/') && !name.contains('\\') {
            let rest = &dest[colon_pos + 1..];
            // Split rest into bucket + optional prefix.
            let (bucket, prefix) = match rest.find('/') {
                Some(slash) => (rest[..slash].to_string(), rest[slash + 1..].to_string()),
                None => (rest.to_string(), String::new()),
            };
            if !bucket.is_empty() {
                return Destination::S3 {
                    remote_name: name.to_string(),
                    bucket,
                    prefix,
                };
            }
        }
    }
    Destination::Local(std::path::PathBuf::from(dest))
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub async fn run() {
    // Intercept the `config` subcommand before clap parses the main args.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("config") {
        run_config_cmd(&raw[1..]).await;
        return;
    }

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

    match parse_destination(&args.destination) {
        Destination::Local(dest_path) => {
            run_local_copy(args, dest_path, parallel, is_quiet).await;
        }
        Destination::S3 {
            remote_name,
            bucket,
            prefix,
        } => {
            run_s3_upload(args, remote_name, bucket, prefix, parallel, is_quiet).await;
        }
    }
}

// ─── Local copy ───────────────────────────────────────────────────────────────

async fn run_local_copy(
    args: Args,
    destination: std::path::PathBuf,
    parallel: usize,
    is_quiet: bool,
) {
    if !destination.exists() {
        log::debug!("Destination path does not exist: {}", destination.display());
        println!(
            "{} {}",
            "Destination path does not exist: ".red(),
            destination.display().to_string().red()
        );
        std::process::exit(1);
    }
    if !destination.is_dir() {
        log::debug!(
            "Destination path is not a directory: {}",
            destination.display()
        );
        println!(
            "{} {}",
            "Destination path is not a directory: ".red(),
            destination.display().to_string().red()
        );
        std::process::exit(1);
    }

    // Validate sources.
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

    if valid_sources.is_empty() {
        std::process::exit(1);
    }

    // Collect all copy entries.
    let mut all_entries: Vec<CopyEntry> = Vec::new();
    let mut all_dirs: Vec<std::path::PathBuf> = Vec::new();
    let mut total_size: u64 = 0;
    let mut dest_paths: HashSet<std::path::PathBuf> = HashSet::new();

    for source_str in &valid_sources {
        let source = Path::new(source_str);
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

    log::info!(
        "Total files to copy: {}, total size: {}",
        all_entries.len(),
        total_size
    );

    // Create destination directories upfront.
    for dir in &all_dirs {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            eprintln!(
                "{} {}",
                "Error creating directory:".red(),
                e.to_string().red()
            );
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
    let sync = args.sync;
    let mut tasks = Vec::new();

    for entry in all_entries {
        let sem = Arc::clone(&semaphore);
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
                sync,
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

// ─── S3 upload ────────────────────────────────────────────────────────────────

async fn run_s3_upload(
    args: Args,
    remote_name: String,
    bucket: String,
    prefix: String,
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

        match s3::collect_s3_upload_entries(source, &prefix).await {
            Ok((entries, _count, size)) => {
                all_entries.extend(entries);
                total_size += size;
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
        task.await.expect("upload task failed");
    }

    if let Some(pb) = main_pb {
        pb.finish_with_message("Upload complete!");
    }

    if *has_failed.lock().unwrap() {
        std::process::exit(1);
    }
}

// ─── Config subcommand ────────────────────────────────────────────────────────

async fn run_config_cmd(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("create") => match args.get(1) {
            Some(name) => config_create(name),
            None => {
                eprintln!("Usage: cp2 config create <name>");
                std::process::exit(1);
            }
        },
        Some("list") => config_list(),
        Some("delete") => match args.get(1) {
            Some(name) => config_delete(name),
            None => {
                eprintln!("Usage: cp2 config delete <name>");
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!(
                "Usage:\n  cp2 config create <name>   Create a new remote configuration\n  cp2 config list            List all configured remotes\n  cp2 config delete <name>   Remove a remote configuration"
            );
            std::process::exit(1);
        }
    }
}

fn prompt(label: &str, default: &str) -> String {
    if default.is_empty() {
        print!("{}: ", label);
    } else {
        print!("{} [{}]: ", label, default);
    }
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap_or(0);
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed
    }
}

fn config_create(name: &str) {
    println!("Creating remote \"{}\"", name);
    println!("Only S3-compatible remotes are supported.\n");

    let provider = prompt("Provider (e.g. AWS, Minio, DigitalOcean)", "AWS");
    let access_key_id = prompt("Access key ID", "");
    let secret_access_key = rpassword::prompt_password("Secret access key: ").unwrap_or_default();
    let region = prompt("Region", "us-east-1");
    let endpoint = prompt("Endpoint URL (leave blank for AWS S3)", "");

    let mut cfg = config::load_config().unwrap_or_default();

    cfg.insert(
        name.to_string(),
        RemoteConfig {
            remote_type: "s3".to_string(),
            provider: Some(provider),
            access_key_id: Some(access_key_id),
            secret_access_key: Some(secret_access_key),
            region: Some(region),
            endpoint: if endpoint.is_empty() {
                None
            } else {
                Some(endpoint)
            },
        },
    );

    match config::save_config(&cfg) {
        Ok(_) => println!(
            "\nRemote \"{}\" saved to {}",
            name,
            config::config_path().display()
        ),
        Err(e) => {
            eprintln!("Failed to save configuration: {}", e);
            std::process::exit(1);
        }
    }
}

fn config_list() {
    let cfg = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    if cfg.is_empty() {
        println!("No remotes configured. Use `cp2 config create <name>` to add one.");
        return;
    }

    let mut names: Vec<&String> = cfg.keys().collect();
    names.sort();
    for name in names {
        let r = &cfg[name];
        let provider = r.provider.as_deref().unwrap_or("unknown");
        let region = r.region.as_deref().unwrap_or("unknown");
        println!(
            "{:<20} type={} provider={} region={}",
            name, r.remote_type, provider, region
        );
    }
}

fn config_delete(name: &str) {
    let mut cfg = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    if cfg.remove(name).is_none() {
        eprintln!("Remote \"{}\" not found.", name);
        std::process::exit(1);
    }

    match config::save_config(&cfg) {
        Ok(_) => println!("Remote \"{}\" deleted.", name),
        Err(e) => {
            eprintln!("Failed to save configuration: {}", e);
            std::process::exit(1);
        }
    }
}
