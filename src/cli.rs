use crate::{cmd_config, cmd_local, cmd_s3};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_verbosity_flag::Verbosity;
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

    /// Destination directory (local path or remote:bucket/prefix)
    #[arg(required = true)]
    destination: Option<String>,

    /// Enable recursive copying for directories
    #[arg(short, long, default_value_t = false)]
    recursive: bool,

    /// Parallel level (number of concurrent copy operations)
    #[arg(short, long, default_value_t = 4)]
    parallel: usize,

    /// Sync each file to disk after copying (slower, but crash-safe; local copies only)
    #[arg(short = 'S', long, default_value_t = false)]
    sync: bool,

    #[command(flatten)]
    verbosity: Verbosity,
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
    },
    /// List all configured remotes
    List,
    /// Remove a remote configuration
    Delete {
        /// Name of the remote to delete
        name: String,
    },
}

impl Args {
    fn command_with_dynamic_parallel() -> clap::Command {
        let max = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        Args::command()
            .mut_arg("parallel", move |arg| arg.help(format!("Parallel level (max: {max})")))
    }
}

// ─── Destination type ─────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum Destination {
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
pub fn parse_destination(dest: &str) -> Destination {
    if let Some(colon_pos) = dest.find(':') {
        let name = &dest[..colon_pos];
        // A remote name must be at least 2 characters and must not contain
        // path separators. The 2-char minimum disambiguates remotes from
        // Windows drive letters like `C:\path`.
        if name.len() >= 2 && !name.contains('/') && !name.contains('\\') {
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
    let max = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    log::debug!("Max parallel level (number of CPU cores): {}", max);
    let matches = Args::command_with_dynamic_parallel().get_matches();
    let args = Args::from_arg_matches(&matches).expect("parse args");
    log::debug!("Parsed args: {:#?}", args);

    env_logger::Builder::new()
        .filter_level(args.verbosity.into())
        .init();

    if let Some(Command::Config { action }) = args.command {
        cmd_config::run(action);
        return;
    }

    let parallel = args.parallel.min(max);
    log::debug!("Using parallel level: {}", parallel);

    let is_quiet = args.verbosity.is_silent();

    // Required by clap when no subcommand is used.
    let destination = args
        .destination
        .expect("clap guarantees destination is set when no subcommand is used");

    match parse_destination(&destination) {
        Destination::Local(dest_path) => {
            cmd_local::run(
                args.source,
                dest_path,
                args.recursive,
                args.sync,
                parallel,
                is_quiet,
            )
            .await;
        }
        Destination::S3 {
            remote_name,
            bucket,
            prefix,
        } => {
            cmd_s3::run(
                args.source,
                remote_name,
                bucket,
                prefix,
                args.recursive,
                parallel,
                is_quiet,
            )
            .await;
        }
    }
}
