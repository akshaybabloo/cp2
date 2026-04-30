use crate::{cmd_config, cmd_local, cmd_s3};
use clap::{CommandFactory, FromArgMatches, Parser};
use clap_verbosity_flag::Verbosity;
use std::thread;

// ─── CLI arguments ────────────────────────────────────────────────────────────

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
        cmd_config::run(&raw[1..]).await;
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
