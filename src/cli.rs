use crate::{cmd_config, cmd_local, cmd_s3, cmd_sftp, config};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_verbosity_flag::{Verbosity, WarnLevel};
use colored::Colorize;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;
use std::thread;

// ─── CLI arguments ────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Source files or directories
    #[arg(required = true)]
    source: Vec<String>,

    /// Destination directory: a local path, <remote>:<bucket>/<prefix> for a
    /// configured S3 remote, or <host>:<path> for SFTP over ssh
    #[arg(required = true)]
    destination: Option<String>,

    /// Enable recursive copying for directories
    #[arg(short, long, default_value_t = false)]
    recursive: bool,

    /// Parallel level (number of concurrent copy operations)
    #[arg(short, long, default_value_t = 4, value_parser = parse_parallel)]
    parallel: usize,

    /// Sync each file to disk after copying (slower, but crash-safe; local copies only)
    #[arg(short = 'S', long, default_value_t = false)]
    sync: bool,

    #[command(flatten)]
    verbosity: Verbosity<WarnLevel>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Manage remote configurations
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ConfigAction {
    /// Create a new remote configuration
    Create {
        /// Name of the remote (e.g. "myaws")
        name: String,
        /// Overwrite an existing remote of the same name without prompting
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
    /// List all configured remotes
    List,
    /// Remove a remote configuration
    Delete {
        /// Name of the remote to delete
        name: String,
        /// Skip the confirmation prompt
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
}

impl Args {
    fn command_with_dynamic_parallel() -> clap::Command {
        let max = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        Args::command()
            .mut_arg("parallel", move |arg| {
                arg.help(format!(
                    "Parallel level (local copies are capped at {max}, the CPU count)"
                ))
            })
            // Short flag only: -v lists each file, -vv adds debug output.
            .mut_arg("verbose", |arg| {
                arg.long(None)
                    .help("List each file as it is copied (-vv for debug output)")
                    .long_help(None)
            })
    }
}

fn parse_parallel(s: &str) -> Result<usize, String> {
    let n: usize = s
        .parse()
        .map_err(|_| format!("`{s}` is not a valid positive integer"))?;
    if n == 0 {
        return Err("must be at least 1".to_string());
    }
    Ok(n)
}

// ─── Destination type ─────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum Destination {
    /// A regular local filesystem path.
    Local(std::path::PathBuf),
    /// `name:path`. `name` is a configured remote (S3), or otherwise an SSH
    /// host as understood by `ssh` (an alias from `~/.ssh/config` or
    /// `user@host`).
    Remote { name: String, path: String },
}

/// Parses a destination string, distinguishing `name:path` from a plain
/// local path.
pub fn parse_destination(dest: &str) -> Destination {
    if let Some((name, path)) = dest.split_once(':') {
        // A remote name must be at least 2 characters and must not contain
        // path separators. The 2-char minimum disambiguates remotes from
        // Windows drive letters like `C:\path`. A path starting with `//`
        // is a URL (`http://...`), not a remote.
        if name.len() >= 2 && !name.contains('/') && !name.contains('\\') && !path.starts_with("//") {
            return Destination::Remote {
                name: name.to_string(),
                path: path.to_string(),
            };
        }
    }
    Destination::Local(std::path::PathBuf::from(dest))
}

/// Splits an S3 remote path into its bucket and key prefix. Returns `None`
/// when the bucket is missing.
pub fn split_bucket(path: &str) -> Option<(String, String)> {
    let (bucket, prefix) = path.split_once('/').unwrap_or((path, ""));
    if bucket.is_empty() {
        return None;
    }
    Some((bucket.to_string(), prefix.to_string()))
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub async fn run() {
    let matches = Args::command_with_dynamic_parallel().get_matches();
    let args = Args::from_arg_matches(&matches).expect("parse args");

    // Log lines go through the progress bars' MultiProgress so they don't
    // corrupt the bars while they are drawn.
    let multi = MultiProgress::new();
    let logger = env_logger::Builder::new().filter_level(args.verbosity.into()).build();
    let level = logger.filter();
    LogWrapper::new(multi.clone(), logger)
        .try_init()
        .expect("logger is initialised once");
    log::set_max_level(level);

    let max = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    log::debug!("Max parallel level (number of CPU cores): {}", max);
    log::debug!("Parsed args: {:#?}", args);

    if let Some(Command::Config { action }) = args.command {
        cmd_config::run(action);
        return;
    }

    // Warnings are shown by default, so any -q hides the progress bars.
    let is_quiet = args.verbosity.log_level_filter() < log::LevelFilter::Warn;
    let multi = (!is_quiet).then_some(multi);

    // Required by clap when no subcommand is used.
    let destination = args
        .destination
        .expect("clap guarantees destination is set when no subcommand is used");

    match parse_destination(&destination) {
        Destination::Local(dest_path) => {
            // Local copies are bounded by the machine.
            let parallel = args.parallel.clamp(1, max);
            log::debug!("Using parallel level: {}", parallel);
            cmd_local::run(args.source, dest_path, args.recursive, args.sync, parallel, multi).await;
        }
        Destination::Remote { name, path } => {
            // Uploads are bounded by the network, not the CPU count.
            log::debug!("Using parallel level: {}", args.parallel);
            let cfg = match config::load_config() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{} {}", "Failed to load configuration:".red(), e);
                    std::process::exit(1);
                }
            };
            match config::get_remote(&cfg, &name) {
                Some(remote) => {
                    let Some((bucket, prefix)) = split_bucket(&path) else {
                        eprintln!(
                            "{} {}",
                            "S3 destinations need a bucket:".red(),
                            format!("{name}:<bucket>/<prefix>").red()
                        );
                        std::process::exit(1);
                    };
                    cmd_s3::run(
                        args.source,
                        remote.clone(),
                        bucket,
                        prefix,
                        args.recursive,
                        args.parallel,
                        multi,
                    )
                    .await;
                }
                None => {
                    log::debug!("No remote named {name:?}; using SFTP over ssh");
                    cmd_sftp::run(args.source, name, path, args.recursive, args.parallel, multi).await;
                }
            }
        }
    }
}
