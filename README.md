# cp2

`cp2` is a CLI tool to copy files and folders to a destination with a progress bar.

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
