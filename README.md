# Hive

Hive is a small local package manager for portable CLI tools on Linux and macOS.

Version 1 is intentionally narrow:

- Manifests are local TOML files
- Each manifest describes exactly one package version
- Multiple installed versions can coexist
- One version at a time is active through Hive-managed symlinks

## Status

This repository currently implements the v1 workflow:

- `hive install <package>`
- `hive list`
- `hive use <package> <version>`
- `hive uninstall <package> <version> [--force]`
- `hive which <package>`

## Build

```bash
cargo build
```

Run the test suite with:

```bash
cargo test
```

## Default Layout

Hive uses user-scoped directories by default:

- Manifests: `~/.config/hive/manifests/`
- Package store: `~/.local/share/hive/pkgs/`
- State: `~/.local/share/hive/state/`
- Shim directory: `~/.local/bin/hive/`

Add `~/.local/bin/hive` to `PATH` if you want active package binaries to be directly executable.

## Usage

Install a package by manifest name:

```bash
hive install rg
```

List installed versions and mark the active one with `*`:

```bash
hive list
```

Switch the active version:

```bash
hive use rg 14.1.0
```

Resolve the active binary path:

```bash
hive which rg
```

Uninstall a version:

```bash
hive uninstall rg 14.0.0
```

Hive refuses to remove the active version unless you pass `--force`:

```bash
hive uninstall rg 14.1.0 --force
```

## Manifest Format

Hive discovers packages from local manifest directories using one of these layouts:

- `<manifest-dir>/<package>.toml`
- `<manifest-dir>/<package>/manifest.toml`

Example manifest:

```toml
name = "rg"
version = "14.1.0"

[platform.linux-x86_64]
url = "https://example.invalid/rg-14.1.0-x86_64.tar.gz"
checksum = "sha256:..."
archive = "tar.gz"
binaries = ["rg"]

[platform.macos-aarch64]
url = "https://example.invalid/rg-14.1.0-aarch64.tar.gz"
checksum = "sha256:..."
archive = "tar.gz"
binaries = ["rg"]
```

Supported platform keys:

- `linux-x86_64`
- `linux-aarch64`
- `macos-x86_64`
- `macos-aarch64`

`binaries` are paths relative to the extracted archive root. Hive verifies that every declared binary exists after extraction.

## Notes and Limits

- Manifest discovery is local-only in v1
- Hive fails on ambiguous manifest matches instead of choosing one
- Archive support is limited to `tar.gz` and `zip`
- No dependency resolution or automatic update discovery is included
