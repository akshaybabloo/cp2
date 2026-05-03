# cp2

`cp2` is a CLI tool to copy files and folders to a destination with a progress bar. It supports both local filesystem copies and uploads to S3-compatible object storage (AWS S3, MinIO, DigitalOcean Spaces, etc.).

## Installation

You can install cp2 using cargo:

```bash
cargo install cp2
```

or download the binary from the [releases page](https://github.com/akshaybabloo/cp2/releases)

## Usage

A single file can be copied to a destination using the following command:

```bash
cp2 <source> <destination>
```

For a multiple files, use:

```bash
cp2 <source1> <source2>... <destination>
```

Progression is shown by default. To disable it, use the `-q` flag:

```bash
cp2 -q <source> <destination>
```

Similar to default `cp` tool, recursive file copy is disabled. You can also use the `-r` flag to copy directories recursively:

```bash
cp2 -r <source_directory> <destination>
```

## S3 Support

`cp2` can upload files and directories to any S3-compatible object storage service.

### Configuring a remote

Before uploading, add an S3 remote with an interactive wizard:

```bash
cp2 config create <name>
```

Example session:

```text
Creating remote "myaws"
Only S3-compatible remotes are supported.

Provider (e.g. AWS, Minio, DigitalOcean) [AWS]: AWS
Access key ID: AKIAIOSFODNN7EXAMPLE
Secret access key:
Region [us-east-1]: eu-west-1
Endpoint URL (leave blank for AWS S3):

Remote "myaws" saved to /home/user/.config/cp2/config.toml
```

The configuration is stored in `~/.config/cp2/config.toml` using TOML format, similar to how rclone stores its remotes.

### Listing configured remotes

```bash
cp2 config list
```

### Deleting a remote

```bash
cp2 config delete <name>
```

### Uploading files to S3

Use the `<remote>:<bucket>/<prefix>` syntax as the destination:

```bash
# Upload a single file to s3://my-bucket/uploads/
cp2 file.txt myaws:my-bucket/uploads

# Upload multiple files
cp2 a.txt b.txt myaws:my-bucket/uploads

# Upload a directory recursively
cp2 -r my-folder myaws:my-bucket/backups
```

Files 8 MiB or larger are automatically uploaded using S3 **multipart upload** for reliability and better throughput.

### S3-compatible services (MinIO, DigitalOcean Spaces, etc.)

Set the `Endpoint URL` during `cp2 config create` to point to any S3-compatible service:

```text
Endpoint URL (leave blank for AWS S3): http://localhost:9000
```

