//! End-to-end SFTP tests. `CP2_SSH` points cp2 at a fake `ssh` that runs a
//! local `sftp-server`, so no SSH server or network is needed. The tests are
//! skipped when no `sftp-server` binary can be found.
#![cfg(unix)]

use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::cargo_bin;
use predicates::prelude::*;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const SFTP_SERVER_CANDIDATES: &[&str] = &[
    "/usr/lib/openssh/sftp-server",
    "/usr/libexec/openssh/sftp-server",
    "/usr/libexec/sftp-server",
    "/usr/lib/ssh/sftp-server",
    "/snap/core24/current/usr/lib/openssh/sftp-server",
];

fn find_sftp_server() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CP2_TEST_SFTP_SERVER") {
        return Some(PathBuf::from(path));
    }
    SFTP_SERVER_CANDIDATES.iter().map(PathBuf::from).find(|p| p.exists())
}

/// Writes a fake `ssh` script into `dir` that ignores its arguments and runs
/// `sftp-server`. Returns `None` (and the test should return early) when no
/// `sftp-server` is installed.
fn fake_ssh(dir: &Path) -> Option<PathBuf> {
    let Some(server) = find_sftp_server() else {
        eprintln!("skipping: no sftp-server found (set CP2_TEST_SFTP_SERVER)");
        return None;
    };
    let script = dir.join("fake-ssh");
    fs::write(&script, format!("#!/bin/sh\nexec '{}'\n", server.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    Some(script)
}

/// A cp2 command using `ssh` and a config file inside `tmp`, so the user's
/// real config (e.g. a remote named `somehost`) can't affect the test.
fn cp2(tmp: &Path, ssh: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("cp2"));
    cmd.env("CP2_SSH", ssh).env("CP2_CONFIG", tmp.join("config.toml"));
    cmd
}

#[test]
fn test_sftp_recursive_upload_keeps_contents_and_metadata() {
    let tmp = TempDir::new().unwrap();
    let Some(ssh) = fake_ssh(tmp.path()) else { return };

    let source = tmp.path().join("project");
    fs::create_dir_all(source.join("sub/deeper")).unwrap();
    File::create(source.join("a.txt")).unwrap().write_all(b"a").unwrap();
    File::create(source.join("sub/deeper/b.txt"))
        .unwrap()
        .write_all(b"bb")
        .unwrap();
    let script = source.join("run.sh");
    File::create(&script).unwrap().write_all(b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o750)).unwrap();
    let mtime = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    File::options()
        .write(true)
        .open(&script)
        .unwrap()
        .set_modified(mtime)
        .unwrap();

    let remote = tmp.path().join("remote");
    fs::create_dir(&remote).unwrap();

    cp2(tmp.path(), &ssh)
        .arg("-r")
        .arg("-p")
        .arg("3")
        .arg(&source)
        .arg(format!("somehost:{}", remote.display()))
        .assert()
        .success();

    let uploaded = remote.join("project");
    assert_eq!(fs::read_to_string(uploaded.join("a.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(uploaded.join("sub/deeper/b.txt")).unwrap(), "bb");
    let meta = fs::metadata(uploaded.join("run.sh")).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o750);
    assert_eq!(meta.modified().unwrap(), mtime);
}

#[test]
fn test_sftp_reupload_over_read_only_file() {
    let tmp = TempDir::new().unwrap();
    let Some(ssh) = fake_ssh(tmp.path()) else { return };

    let source = tmp.path().join("object");
    File::create(&source).unwrap().write_all(b"v1").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o444)).unwrap();
    let remote = tmp.path().join("remote");
    fs::create_dir(&remote).unwrap();

    for _ in 0..2 {
        cp2(tmp.path(), &ssh)
            .arg(&source)
            .arg(format!("somehost:{}", remote.display()))
            .assert()
            .success();
    }
    assert_eq!(fs::read_to_string(remote.join("object")).unwrap(), "v1");
}

#[test]
fn test_sftp_missing_destination_fails() {
    let tmp = TempDir::new().unwrap();
    let Some(ssh) = fake_ssh(tmp.path()) else { return };

    let source = tmp.path().join("file.txt");
    File::create(&source).unwrap().write_all(b"x").unwrap();

    cp2(tmp.path(), &ssh)
        .arg(&source)
        .arg(format!("somehost:{}", tmp.path().join("missing").display()))
        .assert()
        .failure()
        .stderr(predicate::str::contains("destination path"));
}

#[test]
fn test_sftp_reports_ssh_failure() {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("file.txt");
    File::create(&source).unwrap().write_all(b"x").unwrap();

    cp2(tmp.path(), Path::new("false"))
        .arg(&source)
        .arg("somehost:/tmp")
        .assert()
        .failure()
        .stderr(predicate::str::contains("ssh to somehost failed"));
}
