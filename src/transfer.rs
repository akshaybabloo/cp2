//! Plumbing shared by the local copy and S3 upload commands: source
//! validation, progress bars and running transfers in parallel.

use crate::s3::S3UploadEntry;
use crate::utils::{CopyEntry, trim_filename};
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Semaphore;

/// A single file transfer: where it reads from and how many bytes it moves.
pub(crate) trait Transfer: Send + 'static {
    fn source(&self) -> &Path;
    /// Where the file ends up, for display.
    fn destination(&self) -> String;
    fn size(&self) -> u64;
}

impl Transfer for CopyEntry {
    fn source(&self) -> &Path {
        &self.from
    }
    fn destination(&self) -> String {
        self.to.display().to_string()
    }
    fn size(&self) -> u64 {
        self.size
    }
}

impl Transfer for S3UploadEntry {
    fn source(&self) -> &Path {
        &self.from
    }
    fn destination(&self) -> String {
        self.key.clone()
    }
    fn size(&self) -> u64 {
        self.size
    }
}

/// Checks that each source exists and, if it is a directory, that `recursive`
/// is set. Prints an error for every rejected source and returns the accepted
/// ones, plus whether any were rejected.
pub(crate) fn validate_sources(sources: &[String], recursive: bool) -> (Vec<PathBuf>, bool) {
    let mut valid = Vec::new();
    let mut has_errors = false;

    for source_str in sources {
        let source = Path::new(source_str);
        if !source.exists() {
            eprintln!("{} {}", "Source path does not exist:".red(), source_str.red());
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
        valid.push(source.to_path_buf());
    }

    (valid, has_errors)
}

/// The overall progress bar plus the container that per-file bars join.
#[derive(Clone)]
pub(crate) struct Progress {
    multi: MultiProgress,
    main: ProgressBar,
}

impl Progress {
    /// Adds the overall progress bar to `multi`, or returns `None` when there
    /// is no `multi` (quiet mode).
    pub(crate) fn new(multi: Option<MultiProgress>, total_size: u64) -> Option<Self> {
        let multi = multi?;
        let main = multi.add(ProgressBar::new(total_size));
        main.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("=>-"),
        );
        main.set_message("Overall progress");
        main.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(Self { multi, main })
    }

    /// Prints to stderr without corrupting the progress bars.
    fn eprintln(&self, line: &str) {
        self.multi.suspend(|| eprintln!("{}", line));
    }

    pub(crate) fn finish(&self, message: &'static str) {
        self.main.finish_with_message(message);
    }

    fn file_bar(&self, source: &Path, size: u64, verb: &str) -> ProgressBar {
        let file_name = source.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
        let pb = self.multi.add(ProgressBar::new(size));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("  {spinner:.green} {msg:<30} [{wide_bar:.yellow/blue}] {bytes}/{total_bytes}")
                .unwrap()
                .progress_chars("=>-"),
        );
        pb.set_message(format!("{} {}", verb, trim_filename(file_name, 28)));
        pb
    }
}

/// Runs `transfer` on every entry with at most `parallel` running at once.
/// `transfer` receives the entry's own progress bar and the overall bar (both
/// `None` in quiet mode). `verb` (e.g. "Copying") labels the bars and error
/// messages. Returns `true` if any transfer failed.
pub(crate) async fn run_parallel<T, F, Fut>(
    entries: Vec<T>,
    parallel: usize,
    progress: Option<&Progress>,
    verb: &'static str,
    transfer: F,
) -> bool
where
    T: Transfer,
    F: Fn(T, Option<ProgressBar>, Option<ProgressBar>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(parallel));
    let transfer = Arc::new(transfer);
    let failed = Arc::new(AtomicBool::new(false));
    let action = verb.to_lowercase();
    let mut tasks = Vec::new();

    for entry in entries {
        let semaphore = Arc::clone(&semaphore);
        let transfer = Arc::clone(&transfer);
        let failed = Arc::clone(&failed);
        let progress = progress.cloned();
        let action = action.clone();
        let source = entry.source().display().to_string();
        let destination = entry.destination();

        let task = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.expect("failed to acquire semaphore permit");

            let file_pb = progress
                .as_ref()
                .map(|p| p.file_bar(entry.source(), entry.size(), verb));
            let main_pb = progress.as_ref().map(|p| p.main.clone());

            let result = (*transfer)(entry, file_pb.clone(), main_pb).await;
            if let Some(pb) = file_pb {
                pb.finish_and_clear();
            }
            match result {
                // Shown with -v, like `cp -v`.
                Ok(()) => log::info!("{} -> {}", source, destination),
                Err(e) => {
                    report(progress.as_ref(), &format!("Error {} {}: {}", action, source, e));
                    failed.store(true, Ordering::Relaxed);
                }
            }
        });
        tasks.push(task);
    }

    for task in tasks {
        if let Err(e) = task.await {
            report(progress, &format!("{} task failed: {}", verb, e));
            failed.store(true, Ordering::Relaxed);
        }
    }

    failed.load(Ordering::Relaxed)
}

fn report(progress: Option<&Progress>, line: &str) {
    match progress {
        Some(p) => p.eprintln(line),
        None => eprintln!("{}", line),
    }
}
