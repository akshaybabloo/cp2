/// Tests for `cp2::cli::parse_destination`.
use cp2::cli::{parse_destination, Destination};
use std::path::PathBuf;

#[test]
fn local_relative_path() {
    assert_eq!(
        parse_destination("./out"),
        Destination::Local(PathBuf::from("./out")),
    );
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

#[test]
fn s3_remote_with_bucket_only() {
    assert_eq!(
        parse_destination("myaws:my-bucket"),
        Destination::S3 {
            remote_name: "myaws".to_string(),
            bucket: "my-bucket".to_string(),
            prefix: String::new(),
        },
    );
}

#[test]
fn s3_remote_with_prefix() {
    assert_eq!(
        parse_destination("myaws:my-bucket/uploads/2024"),
        Destination::S3 {
            remote_name: "myaws".to_string(),
            bucket: "my-bucket".to_string(),
            prefix: "uploads/2024".to_string(),
        },
    );
}

#[test]
fn empty_bucket_falls_back_to_local() {
    assert_eq!(
        parse_destination("myaws:"),
        Destination::Local(PathBuf::from("myaws:")),
    );
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
