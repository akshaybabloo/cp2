//! SFTP uploads over the system `ssh` client.
//!
//! cp2 runs `ssh -s <host> sftp` and speaks SFTP over the child's stdin and
//! stdout, so the user's `~/.ssh/config`, ssh-agent, `known_hosts`,
//! `ProxyJump` and `ControlMaster` settings all apply unchanged.

use indicatif::ProgressBar;
use russh_sftp::client::{Config, SftpSession};
use russh_sftp::protocol::FileAttributes;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Overrides the `ssh` program, like `GIT_SSH`. Also lets tests run a local
/// `sftp-server` instead of connecting anywhere.
const SSH_PROGRAM_ENV: &str = "CP2_SSH";

/// Time allowed for the SFTP handshake, which only starts once `ssh` has
/// logged in, so it includes any password or host-key prompt.
const HANDSHAKE_TIMEOUT_SECS: u64 = 300;

/// Time allowed for each SFTP request after the handshake.
const REQUEST_TIMEOUT_SECS: u64 = 60;

const BUFFER_SIZE: usize = 256 * 1024;

/// An SFTP session and the `ssh` process carrying it.
pub struct SftpConnection {
    pub session: Arc<SftpSession>,
    child: Child,
}

impl SftpConnection {
    /// Starts `ssh` for `host` and performs the SFTP handshake. `ssh`'s own
    /// prompts and errors go straight to the terminal.
    pub async fn connect(host: &str) -> Result<Self, Error> {
        if host.starts_with('-') {
            return Err(format!("invalid host: {host}").into());
        }
        let program = std::env::var_os(SSH_PROGRAM_ENV).unwrap_or_else(|| "ssh".into());
        let mut child = Command::new(&program)
            .args(["-s", "--", host, "sftp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("could not run {}: {e}", Path::new(&program).display()))?;

        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let config = Config {
            request_timeout_secs: HANDSHAKE_TIMEOUT_SECS,
            ..Config::default()
        };
        let session = match SftpSession::new_with_config(tokio::io::join(stdout, stdin), config).await {
            Ok(s) => s,
            Err(e) => {
                // If ssh itself failed, its own message is already on stderr.
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(format!("ssh to {host} failed ({status}); see the ssh output above").into());
                }
                return Err(format!("could not start an SFTP session on {host}: {e}").into());
            }
        };
        session.set_timeout(REQUEST_TIMEOUT_SECS);

        Ok(Self {
            session: Arc::new(session),
            child,
        })
    }

    /// Ends the SFTP session and waits for `ssh` to exit.
    pub async fn close(mut self) {
        let _ = self.session.close().await;
        let _ = self.child.wait().await;
    }
}

/// Resolves the destination directory on the server and checks that it is a
/// directory. An empty path means the login directory, and a leading `~` is
/// expanded when the server supports it.
pub async fn resolve_base_dir(session: &SftpSession, path: &str) -> Result<String, Error> {
    let mut base = if path.is_empty() {
        ".".to_string()
    } else {
        path.to_string()
    };
    if base.starts_with('~')
        && let Ok(Some(expanded)) = session.expand_path(base.as_str()).await
    {
        base = expanded;
    }
    let meta = session
        .metadata(base.as_str())
        .await
        .map_err(|e| format!("destination path {base}: {e}"))?;
    if !meta.is_dir() {
        return Err(format!("destination path is not a directory: {base}").into());
    }
    Ok(base)
}

/// Creates `dir` unless it already exists as a directory.
pub async fn ensure_dir(session: &SftpSession, dir: &str) -> Result<(), Error> {
    if let Ok(meta) = session.metadata(dir).await {
        if meta.is_dir() {
            return Ok(());
        }
        return Err(format!("{dir} exists on the server and is not a directory").into());
    }
    session
        .create_dir(dir)
        .await
        .map_err(|e| format!("could not create directory {dir}: {e}").into())
}

/// Uploads `from` to the remote path `to`, keeping the source's permissions
/// and access/modification times.
pub async fn upload_file(
    session: &SftpSession,
    from: &Path,
    to: &str,
    file_pb: Option<&ProgressBar>,
    main_pb: Option<&ProgressBar>,
) -> Result<(), Error> {
    let mut source = tokio::fs::File::open(from).await?;
    let source_meta = source.metadata().await?;

    let mut dest = match session.create(to).await {
        Ok(f) => f,
        // An existing read-only file (e.g. from an earlier upload of a
        // read-only source) can't be truncated; replace it instead.
        Err(e) => {
            if !session.try_exists(to).await.unwrap_or(false) {
                return Err(e.into());
            }
            session.remove_file(to).await?;
            session.create(to).await?
        }
    };

    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        let bytes_read = source.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        dest.write_all(&buffer[..bytes_read]).await?;
        if let Some(pb) = file_pb {
            pb.inc(bytes_read as u64);
        }
        if let Some(pb) = main_pb {
            pb.inc(bytes_read as u64);
        }
    }
    // Waits for every pending write to be acknowledged, then closes the file.
    dest.shutdown().await?;

    session.set_metadata(to, attributes_of(&source_meta)).await?;
    Ok(())
}

fn attributes_of(meta: &std::fs::Metadata) -> FileAttributes {
    let secs = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| u32::try_from(d.as_secs()).ok())
    };
    let mut attrs = FileAttributes {
        mtime: secs(meta.modified()),
        atime: secs(meta.accessed()),
        ..FileAttributes::default()
    };
    // SFTP sets both times together, so fall back to mtime for atime.
    if attrs.atime.is_none() {
        attrs.atime = attrs.mtime;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        attrs.permissions = Some(meta.permissions().mode() & 0o7777);
    }
    attrs
}
