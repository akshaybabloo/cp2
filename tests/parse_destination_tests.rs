/// Tests for `cp2::cli::parse_destination`.
use cp2::cli::{Destination, parse_destination, split_bucket};
use std::path::PathBuf;

#[test]
fn local_relative_path() {
    assert_eq!(parse_destination("./out"), Destination::Local(PathBuf::from("./out")),);
}

#[test]
fn local_absolute_path() {
    assert_eq!(
        parse_destination("/tmp/dest"),
        Destination::Local(PathBuf::from("/tmp/dest")),
    );
}

#[test]
fn windows_drive_letter_is_local() {
    // Single-letter "remote" is rejected so Windows paths work.
    assert_eq!(
        parse_destination(r"C:\Users\akshay\downloads"),
        Destination::Local(PathBuf::from(r"C:\Users\akshay\downloads")),
    );
}

#[test]
fn http_url_is_local() {
    // `http:` has empty bucket after split on `/`, so falls back to local.
    assert_eq!(
        parse_destination("http://example.com/foo"),
        Destination::Local(PathBuf::from("http://example.com/foo")),
    );
}

fn remote(name: &str, path: &str) -> Destination {
    Destination::Remote {
        name: name.to_string(),
        path: path.to_string(),
    }
}

#[test]
fn remote_with_bucket_only() {
    assert_eq!(parse_destination("myaws:my-bucket"), remote("myaws", "my-bucket"));
}

#[test]
fn remote_with_prefix() {
    assert_eq!(
        parse_destination("myaws:my-bucket/uploads/2024"),
        remote("myaws", "my-bucket/uploads/2024")
    );
}

#[test]
fn remote_with_empty_path() {
    // `host:` is the login directory for SFTP.
    assert_eq!(parse_destination("myhost:"), remote("myhost", ""));
}

#[test]
fn ssh_user_at_host_with_absolute_path() {
    assert_eq!(
        parse_destination("me@server:/srv/data"),
        remote("me@server", "/srv/data")
    );
}

#[test]
fn split_bucket_with_prefix() {
    assert_eq!(
        split_bucket("my-bucket/uploads/2024"),
        Some(("my-bucket".to_string(), "uploads/2024".to_string()))
    );
}

#[test]
fn split_bucket_only() {
    assert_eq!(
        split_bucket("my-bucket"),
        Some(("my-bucket".to_string(), String::new()))
    );
}

#[test]
fn split_bucket_requires_bucket() {
    assert_eq!(split_bucket(""), None);
    assert_eq!(split_bucket("/abs/path"), None);
}

#[test]
fn empty_remote_name_falls_back_to_local() {
    assert_eq!(
        parse_destination(":bucket"),
        Destination::Local(PathBuf::from(":bucket")),
    );
}

#[test]
fn remote_name_with_slash_falls_back_to_local() {
    assert_eq!(
        parse_destination("foo/bar:baz"),
        Destination::Local(PathBuf::from("foo/bar:baz")),
    );
}
