# Hive

Hive is a small local package manager for portable CLI tools on Linux and macOS.

Version 1 is intentionally narrow:

- Manifests are local TOML files
- Each manifest describes exactly one package version
- Multiple installed versions can coexist
- One version at a time is active through `pkgs/<package>/current` and Hive-managed shims

## Status

This repository currently implements the v1 workflow:

- `hive install <package>`
- `hive list`
- `hive sync <owner>/<repo>`
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
- Package store: `~/.local/share/hive/pkgs/<package>/<version>/`
- State: `~/.local/share/hive/state/`
- Shim directory: `~/.local/bin/hive/`

Each installed package also has a `current` symlink at `~/.local/share/hive/pkgs/<package>/current` that points to the active version directory.

Add `~/.local/bin/hive` to `PATH` if you want active package binaries to be directly executable.

## Usage

Install a package by manifest name:

```bash
hive install rg
```

If the manifest omits binaries for the current platform, `hive install` extracts the archive, lists executable candidates, prompts you to choose one or more, and writes those paths back into the manifest.

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

## Sync A Manifest From GitHub

`hive sync` is interactive:

```bash
hive sync BurntSushi/ripgrep
```

It fetches release metadata, then prompts only for the current platform's asset and binaries. Hive saves the chosen asset filename and binary paths in the manifest, and later syncs reuse those saved values as defaults.

Synced manifests store their source configuration in the manifest itself, along with the generated artifact entry for the current platform:

```toml
name = "ripgrep"
version = "14.1.0"

[source.github]
repo = "BurntSushi/ripgrep"
channel = "stable"

[source.github.platform.macos-aarch64]
asset = "ripgrep-14.1.0-aarch64-apple-darwin.tar.gz"
binaries = ["rg"]

[platform.macos-aarch64]
url = "https://example.invalid/rg-14.1.0-aarch64.tar.gz"
checksum = "sha256:..."
archive = "tar.gz"
binaries = ["rg"]
```

`hive install` still reads local manifests only. `hive sync` updates one local manifest file in place from GitHub release metadata.

## Proxy Environment

Hive resolves proxy settings per process for HTTP and HTTPS requests made by `hive install` and `hive sync`.

- `HIVE_HTTP_PROXY`, then `HTTP_PROXY`, then `http_proxy`
- `HIVE_HTTPS_PROXY`, then `HTTPS_PROXY`, then `https_proxy`
- `HIVE_ALL_PROXY`, then `ALL_PROXY`, then `all_proxy`
- `HIVE_NO_PROXY`, then `NO_PROXY`, then `no_proxy`
- `HIVE_INSECURE_SSL=1` disables TLS certificate verification for all HTTPS requests made by the current Hive process. Use it only in controlled environments such as local testing or corporate interception proxy setups.

Hive-specific variables override the standard ones. Proxy authentication is supported only when credentials are embedded in the proxy URL, for example `http://user:pass@proxy.internal:8080`.

## Manifest Format

Hive discovers packages from local manifest directories using one of these layouts:

- `<manifest-dir>/<package>.toml`
- `<manifest-dir>/<package>/manifest.toml`

Example manifest:

```toml
name = "rg"
version = "14.1.0"

[source.github]
repo = "BurntSushi/ripgrep"
channel = "stable"

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

`binaries` are paths relative to the installed version directory. Hive verifies that every declared binary exists after extraction, and it will normalize a single top-level wrapper directory when the archive extracts that way.

## Notes and Limits

- Manifest discovery is local-only in v1
- Hive fails on ambiguous manifest matches instead of choosing one
- Archive support is limited to `tar.gz`, `tar.xz`, and `zip`
- No dependency resolution or automatic update discovery is included
