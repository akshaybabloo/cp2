use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_sdk_s3::Client;
use indicatif::ProgressBar;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::config::RemoteConfig;

/// Files below this threshold are uploaded with a single PutObject call.
/// Files at or above it use multipart upload.
const MULTIPART_THRESHOLD: u64 = 8 * 1024 * 1024; // 8 MiB

/// Default multipart part size (S3's minimum allowed is 5 MiB).
const MIN_PART_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

/// S3's hard limit on the number of parts per multipart upload.
const MAX_PARTS: u64 = 10_000;

/// S3's hard limit on a single part.
const MAX_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024; // 5 GiB

/// A source file paired with the S3 key it should be uploaded to.
pub struct S3UploadEntry {
    pub from: PathBuf,
    pub key: String,
    pub size: u64,
}

/// Picks a multipart part size such that the file fits within S3's 10,000-part
/// limit. Returns an error if the file exceeds S3's supported maximum (~5 TiB).
#[doc(hidden)]
pub fn pick_part_size(file_size: u64) -> Result<u64, String> {
    let needed = file_size.div_ceil(MAX_PARTS).max(MIN_PART_SIZE);
    if needed > MAX_PART_SIZE {
        return Err(format!(
            "file size {file_size} bytes exceeds S3's maximum supported object size (~5 TiB)"
        ));
    }
    Ok(needed)
}

/// Builds an [`aws_sdk_s3::Client`] from a [`RemoteConfig`].
pub async fn create_client(
    config: &RemoteConfig,
) -> Result<Client, Box<dyn std::error::Error + Send + Sync>> {
    let access_key = config.access_key_id.as_deref().unwrap_or_default().to_string();
    let secret_key = config
        .secret_access_key
        .as_deref()
        .unwrap_or_default()
        .to_string();

    let credentials = Credentials::new(access_key, secret_key, None, None, "cp2");

    let region_str = config
        .region
        .clone()
        .unwrap_or_else(|| "us-east-1".to_string());
    let region = aws_config::Region::new(region_str);

    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(region)
        .credentials_provider(credentials)
        .load()
        .await;

    let s3_builder = aws_sdk_s3::config::Builder::from(&sdk_config);

    let s3_config = match config.endpoint.as_deref() {
        Some(ep) if !ep.is_empty() => s3_builder
            .endpoint_url(ep)
            .force_path_style(true)
            .build(),
        _ => s3_builder.build(),
    };

    Ok(Client::from_conf(s3_config))
}

/// Collects all files that should be uploaded, mapping each source path to its
/// S3 key.
///
/// - For a single file `foo.txt` with `key_prefix = "uploads"` the key is
///   `uploads/foo.txt`.
/// - For a directory `mydir` with `key_prefix = "uploads"` each file is keyed
///   as `uploads/mydir/<relative-path>`.
pub async fn collect_s3_upload_entries(
    source: &Path,
    key_prefix: &str,
) -> Result<(Vec<S3UploadEntry>, u64, u64), Box<dyn std::error::Error>> {
    let mut entries = Vec::new();
    let mut total_count = 0u64;
    let mut total_size = 0u64;

    // Normalise prefix: ensure it ends with '/' when non-empty so we can
    // simply concatenate keys below.
    let prefix = if key_prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", key_prefix.trim_end_matches('/'))
    };

    let meta = tokio::fs::symlink_metadata(source).await?;

    if meta.file_type().is_file() {
        let file_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("source has no file name")?;
        let key = format!("{}{}", prefix, file_name);
        let size = meta.len();
        entries.push(S3UploadEntry {
            from: source.to_path_buf(),
            key,
            size,
        });
        return Ok((entries, 1, size));
    }

    if meta.file_type().is_dir() {
        let dir_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("source has no directory name")?;

        let mut stack = vec![source.to_path_buf()];
        while let Some(p) = stack.pop() {
            let m = tokio::fs::symlink_metadata(&p).await?;
            if m.file_type().is_dir() {
                let mut dir_entries = tokio::fs::read_dir(&p).await?;
                while let Some(entry) = dir_entries.next_entry().await? {
                    stack.push(entry.path());
                }
            } else if m.file_type().is_file() {
                let relative = p.strip_prefix(source)?;
                // Convert path separators to '/' for S3 keys.
                let rel_str = relative
                    .to_str()
                    .ok_or("non-UTF-8 path")?
                    .replace('\\', "/");
                let key = format!("{}{}/{}", prefix, dir_name, rel_str);
                let size = m.len();
                total_count += 1;
                total_size += size;
                entries.push(S3UploadEntry {
                    from: p,
                    key,
                    size,
                });
            }
            // Symlinks and special file types are skipped.
        }
    }

    Ok((entries, total_count, total_size))
}

/// Uploads a single file to S3, choosing between a simple PutObject and a
/// multipart upload based on the file size.
pub async fn upload_file(
    client: &Client,
    from: &Path,
    bucket: &str,
    key: &str,
    file_pb: Option<&ProgressBar>,
    main_pb: Option<&ProgressBar>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file_size = tokio::fs::metadata(from).await?.len();

    if file_size < MULTIPART_THRESHOLD {
        upload_single(client, from, bucket, key, file_size, file_pb, main_pb).await
    } else {
        let part_size = pick_part_size(file_size)?;
        upload_multipart(client, from, bucket, key, part_size, file_pb, main_pb).await
    }
}

/// Uploads a file using a single PutObject request.
async fn upload_single(
    client: &Client,
    from: &Path,
    bucket: &str,
    key: &str,
    file_size: u64,
    file_pb: Option<&ProgressBar>,
    main_pb: Option<&ProgressBar>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let body = ByteStream::from_path(from).await?;

    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(body)
        .send()
        .await?;

    if let Some(pb) = file_pb {
        pb.inc(file_size);
    }
    if let Some(pb) = main_pb {
        pb.inc(file_size);
    }

    Ok(())
}

/// Uploads a file using S3 multipart upload, reporting progress after each
/// part.
async fn upload_multipart(
    client: &Client,
    from: &Path,
    bucket: &str,
    key: &str,
    part_size: u64,
    file_pb: Option<&ProgressBar>,
    main_pb: Option<&ProgressBar>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initiate multipart upload.
    let create_resp = client
        .create_multipart_upload()
        .bucket(bucket)
        .key(key)
        .send()
        .await?;

    let upload_id = create_resp
        .upload_id()
        .ok_or("S3 did not return an upload ID")?
        .to_string();

    let mut file = File::open(from).await?;
    let part_size_usize: usize = part_size
        .try_into()
        .map_err(|_| "part size does not fit in usize on this platform")?;
    let mut buf = vec![0u8; part_size_usize];
    let mut completed_parts: Vec<CompletedPart> = Vec::new();
    let mut part_number = 1i32;

    loop {
        let bytes_read = read_at_least(&mut file, &mut buf).await?;
        if bytes_read == 0 {
            break;
        }

        let data = buf[..bytes_read].to_vec();
        let chunk_len = bytes_read as u64;

        let part_result = client
            .upload_part()
            .bucket(bucket)
            .key(key)
            .upload_id(&upload_id)
            .part_number(part_number)
            .body(ByteStream::from(data))
            .send()
            .await;

        match part_result {
            Ok(resp) => {
                let etag = resp.e_tag().unwrap_or_default().to_string();
                completed_parts.push(
                    CompletedPart::builder()
                        .part_number(part_number)
                        .e_tag(etag)
                        .build(),
                );

                if let Some(pb) = file_pb {
                    pb.inc(chunk_len);
                }
                if let Some(pb) = main_pb {
                    pb.inc(chunk_len);
                }
            }
            Err(e) => {
                // Best-effort abort so the incomplete upload doesn't incur
                // storage costs.
                let _ = client
                    .abort_multipart_upload()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                return Err(e.into());
            }
        }

        part_number += 1;
    }

    let completed = CompletedMultipartUpload::builder()
        .set_parts(Some(completed_parts))
        .build();

    client
        .complete_multipart_upload()
        .bucket(bucket)
        .key(key)
        .upload_id(&upload_id)
        .multipart_upload(completed)
        .send()
        .await?;

    Ok(())
}

/// Reads up to `buf.len()` bytes from `file`, filling the buffer as much as
/// possible before returning.  Returns 0 only at EOF.
async fn read_at_least(
    file: &mut File,
    buf: &mut [u8],
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let mut total = 0;
    while total < buf.len() {
        let n = file.read(&mut buf[total..]).await?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}
