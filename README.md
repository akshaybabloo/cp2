# cp2

`cp2` is a CLI tool to copy files and folders to a destination with a progress bar. It supports local filesystem copies, uploads to other machines over SFTP (using your system's OpenSSH client), and uploads to S3-compatible object storage (AWS S3, MinIO, DigitalOcean Spaces, etc.).

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

## SFTP Support

> [!NOTE]
> `cp2` relies on your system's `ssh` client for SFTP connections. Install OpenSSH if it is not already available on your system.

`cp2` can upload files and directories to any machine running an OpenSSH server (`openssh-server`), which has SFTP enabled by default. It uses your system's `ssh` client to connect, so everything you have already set up for `ssh` works unchanged:

- host aliases, `User`, `Port` and `IdentityFile` from `~/.ssh/config`
- ssh-agent and hardware or 1Password SSH keys
- `ProxyJump` / bastion hosts
- `known_hosts` checks and the usual host-key prompt
- connection sharing with `ControlMaster`

No `cp2 config` setup is needed. Use `<host>:<path>` as the destination, where `<host>` is anything `ssh` accepts:

```bash
# Upload a file to /srv/backups on a host from ~/.ssh/config
cp2 file.txt myserver:/srv/backups

# user@host works too; a relative path is relative to the login directory
cp2 a.txt b.txt me@192.168.1.20:uploads

# Upload a directory recursively into the login directory
cp2 -r my-folder myserver:
```

The destination directory must already exist; subdirectories are created as needed. Uploaded files keep their permissions and modification times.

With `-p`, `cp2` opens one SSH connection per parallel upload, which also avoids the throughput limit of a single SSH connection on slow or long-distance links. If you log in with a password rather than a key, you will be asked once per connection; use ssh-agent or `ControlMaster` to avoid that.

If a name is both a configured S3 remote and an SSH host, the S3 remote wins.

To use a different `ssh` program, set `CP2_SSH`, e.g. `CP2_SSH=/usr/local/bin/ssh`.

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

