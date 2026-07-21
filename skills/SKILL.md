---
name: "siwi-download"
description: "Use siwi-download CLI tool for downloading files with breakpoint continuation and progress tracking. Invoke when needing to download files from HTTP/HTTPS URLs, especially large files that may need resume support."
---

# Siwi Download

Use the siwi-download CLI tool for high-performance file downloads with breakpoint continuation support.

## When to Use

Invoke this skill when:
- Downloading files from HTTP/HTTPS URLs via command line
- Needing to download large files that may be interrupted
- Requiring progress tracking during downloads
- Downloading through HTTP/HTTPS proxies
- Needing custom output directories or filenames for downloads
- Building scripts that need reliable file download capabilities

## Core Features

- **Async Download**: Built on Tokio for high-performance downloads
- **Progress Tracking**: Visual progress bar with download speed and ETA
- **Resume Support**: Breakpoint continuation for interrupted downloads
- **Proxy Support**: HTTP/HTTPS proxy support
- **Flexible Storage**: Custom output directory and filename support
- **JSON Output**: Machine-readable report output for scripting

## Installation

```bash
# Install via cargo
cargo install siwi-download

# Verify installation
siwi-download --version
```

## CLI Usage

### Basic Syntax

```bash
siwi-download <URL> [OPTIONS]
```

### Command Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `<url>` | positional | URL to download (first argument) | Required |
| `--url` | `-u` | URL to download (alternative) | - |
| `--output` | `-o` | Output directory | Current directory |
| `--filename` | `-f` | Custom filename | Auto-extracted from URL |
| `--progress` | `-P` | Show progress bar | `false` |
| `--proxy` | `-p` | HTTP proxy URL | None |
| `--verbose` | `-v` | Verbose logging | `false` |
| `--json` | `-j` | Output report in JSON format | `false` |
| `--help` | `-h` | Show help | - |
| `--version` | `-V` | Show version | - |

### Usage Examples

**Basic download:**
```bash
siwi-download https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg
```

**Download with URL flag:**
```bash
siwi-download -u https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg
```

**Download to specific directory with progress:**
```bash
siwi-download https://example.com/file.zip -o /tmp/downloads -P
```

**Download with custom filename:**
```bash
siwi-download https://example.com/download -f my-custom-name.zip
```

**Download through proxy:**
```bash
siwi-download https://large-file.iso -p http://127.0.0.1:7890 -P
```

**Verbose mode for debugging:**
```bash
siwi-download https://example.com/file.zip -v
```

**JSON output for scripting:**
```bash
siwi-download https://example.com/file.zip -j
```

**Combined options:**
```bash
siwi-download https://example.com/large-file.zip -o ./downloads -f backup.zip -p http://127.0.0.1:7890 -P -v
```

## Output Format

### Standard Output

```
Downloading: https://example.com/file.zip
File: file.zip
Size: 1048576 bytes
[████████████████████████████████████████] 100% (1.00 MB/s)
Download completed: /downloads/file.zip
Time used: 5s
```

### JSON Output

Use `-j` flag for machine-readable output:

```json
{
  "url": "https://example.com/file.zip",
  "file_name": "file.zip",
  "origin_file_name": "file.zip",
  "storage_path": "/downloads",
  "file_path": "/downloads/file.zip",
  "file_size": 1048576,
  "download_status": "Complete",
  "head_status": 200,
  "resp_status": 206,
  "time_used": 5
}
```

## Download Status Types

The `download_status` field indicates how the download was completed:

- **Complete**: Downloaded the entire file in one session
- **Create**: Created a new file (first time download)
- **Append**: Resumed and appended to an existing partial file

## Use Cases

### 1. Downloading Large Files

For large files that may take a long time to download:

```bash
siwi-download https://example.com/large-file.iso -P
```

If interrupted, simply run the same command again to resume:

```bash
siwi-download https://example.com/large-file.iso -P
```

### 2. Batch Downloads in Scripts

Create a script to download multiple files:

```bash
#!/bin/bash
urls=(
  "https://example.com/file1.zip"
  "https://example.com/file2.zip"
  "https://example.com/file3.zip"
)

for url in "${urls[@]}"; do
  siwi-download "$url" -o ./downloads -j
done
```

### 3. Downloading Through Proxy

When behind a corporate firewall or using a VPN:

```bash
siwi-download https://example.com/file.zip -p http://proxy.company.com:8080 -P
```

### 4. Custom File Organization

Organize downloads with custom filenames:

```bash
siwi-download https://example.com/v1.2.3/release -f myapp-v1.2.3.tar.gz -o ./releases
```

## Troubleshooting

### Connection Issues

Enable verbose mode to see detailed logs:

```bash
siwi-download https://example.com/file.zip -v
```

### Proxy Configuration

If downloads fail, try using a proxy:

```bash
siwi-download https://example.com/file.zip -p http://127.0.0.1:7890
```

### File Already Exists

If a partial download exists, the tool will automatically resume:

```bash
siwi-download https://example.com/file.zip -P
# Will resume from where it left off
```

## Best Practices

1. **Always use `-P` for large files** to see progress
2. **Use `-v` when debugging** connection issues
3. **Specify output directory** with `-o` for better organization
4. **Use custom filenames** with `-f` for better file management
5. **Enable JSON output** with `-j` when scripting
6. **Set proxy** with `-p` when behind firewalls

## Building from Source

If you need to build from source:

```bash
# Clone the repository
git clone https://github.com/rs-videos/siwi-download.git
cd siwi-download

# Build the project
cargo build --release

# Install locally
cargo install --path .
```

## Development Commands

For contributors or developers:

```bash
# Build the project
cargo build --verbose

# Run tests
cargo test --verbose

# Run specific test
cargo test test_name -- --nocapture

# Format code
cargo fmt

# Check formatting
cargo fmt --check

# Lint with clippy
cargo clippy
```

## Key Technical Details

- **Async Runtime**: Tokio for high-performance async I/O
- **HTTP Client**: Reqwest with rustls for secure connections
- **Progress Display**: indicatif for beautiful progress bars
- **Error Handling**: anyhow for comprehensive error reporting
- **Logging**: tracing for structured logging
- **Date/Time**: chrono for timestamp handling
