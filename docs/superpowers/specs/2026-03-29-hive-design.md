# Hive v1 Design

## Summary

Hive is a Scoop-like package manager for Linux and macOS focused on installing portable CLI tools from local manifests. Version 1 is intentionally narrow: manifests are local-only, each manifest pins one exact package version, multiple versions of the same package may be installed side by side, and one version at a time may be active in Hive's own shim directory under the user's home directory.

## Goals

- Install portable CLI tools from local manifests using package names such as `hive install rg`
- Support Linux and macOS on common `x86_64` and `aarch64` platforms
- Keep multiple installed versions of a package in an isolated local store
- Let the user switch the active version of a package without reinstalling
- Keep all operations in user-owned directories without requiring root

## Non-Goals

- Remote manifest buckets or repo sync
- Automatic "latest" version discovery
- GUI app bundle installation
- Package dependency resolution
- System-wide installation

## User Experience

### Primary Commands

- `hive install <package>`
- `hive list`
- `hive use <package> <version>`
- `hive uninstall <package> <version>`
- `hive which <package>`

### Install Flow

When the user runs `hive install rg`, Hive searches configured local manifest directories for a manifest that declares the package `rg`. Once resolved, Hive selects the artifact for the current platform, downloads it, verifies its checksum, extracts it, and installs it into the versioned package store. If the package has no active version yet, the installed version becomes active automatically.

### Version Handling

Hive stores each package version in its own directory. Multiple versions may coexist. Only one installed version is active at a time for a given package. Activating a version updates symlinks in Hive's shim directory so the package's exported command names resolve to the selected version.

`hive uninstall <package> <version>` removes one installed version. It must refuse to remove the active version unless the user explicitly forces the action or switches to another installed version first.

## Architecture

Hive should be implemented as a Rust CLI with four core layers.

### CLI Layer

Responsible for parsing commands and arguments, rendering user-facing output, and invoking application services.

### Manifest Layer

Responsible for discovering local manifests by package name, parsing manifest files, validating required fields, and resolving the correct platform-specific artifact definition.

### Installer Layer

Responsible for downloading artifacts, verifying checksums, extracting archives, arranging extracted files into the package store, and cleaning up partial installs after failure.

### Activation Layer

Responsible for tracking the active version of each package and creating or updating symlinks in Hive's shim directory.

## Filesystem Layout

Hive should default to user-scoped directories:

- Manifest directories: `~/.config/hive/manifests/`
- Package store: `~/.local/share/hive/pkgs/<name>/<version>/`
- State directory: `~/.local/share/hive/state/`
- Hive shim directory: `~/.local/bin/hive/`

Paths should be configurable so tests can run against temporary directories instead of the real home directory.
Users must add `~/.local/bin/hive` to `PATH` to invoke active package binaries directly.

## Manifest Discovery

Version 1 uses local manifests only. The CLI accepts package names rather than manifest file paths. Hive searches one or more configured manifest directories for a package manifest. If no manifest is found, installation fails. If multiple manifests resolve to the same package name, Hive fails with an ambiguity error rather than picking one implicitly.

The first version should keep discovery rules simple and deterministic. A package may be represented by either:

- `<manifest-dir>/<package>.toml`
- `<manifest-dir>/<package>/manifest.toml`

Each manifest declares exactly one package version.

## Manifest Format

The manifest format should be TOML. Each manifest must include:

- Package name
- Exact version
- Per-platform artifact URL
- Per-platform checksum
- Per-platform archive type
- Per-platform exported binary paths relative to the extracted root

Platform keys for v1:

- `linux-x86_64`
- `linux-aarch64`
- `macos-x86_64`
- `macos-aarch64`

An example shape:

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

The binary paths are interpreted relative to the extracted artifact root. Hive should validate that each declared binary exists after extraction and fail the install if any are missing.

## Install and Activation Model

Each successful install lands in a versioned directory in the package store. The package store is the source of truth for installed payloads. A small state file under the state directory records active-version mappings and install metadata needed for listing and uninstall checks.

Activation is separate from installation. If a package has no active version, the first installed version becomes active automatically. Otherwise, the current active version remains unchanged until the user runs `hive use <package> <version>`.

Activation updates symlinks in the Hive shim directory for every exported command of the package. In v1, one active version per package owns those command names within that directory.

## Error Handling

Hive must fail closed in these cases:

- Manifest not found
- Ambiguous manifest resolution
- Unsupported current platform
- Missing checksum
- Checksum mismatch
- Download failure
- Unsupported or malformed archive
- Declared binary path missing after extraction

Failed installs must clean up temporary files and incomplete package-store directories so Hive never leaves a half-installed version behind.

## Testing Strategy

### Unit Tests

- Manifest parsing and validation
- Platform resolution
- Checksum parsing and verification
- Active-version state transitions
- Symlink target selection

### Integration Tests

- Install a fixture archive into temporary directories
- Verify package-store layout
- Verify automatic activation for the first install
- Verify switching active versions
- Verify refusal to uninstall the active version without explicit force

### Platform-Focused Tests

- Archive handling for zip and tar-based formats
- Symlink behavior in the Hive shim directory on Linux and macOS

Tests should inject all directories through configuration so they do not depend on or mutate the developer's real home directory.

## Suggested Internal Modules

- `cli`
- `config`
- `manifest`
- `installer`
- `activation`
- `state`
- `platform`
- `fs`

This module split is meant to keep boundaries clear rather than force a particular file layout.

## Open Design Decisions Deferred Past v1

- Adding local manifest search precedence rules beyond "fail on ambiguity"
- Supporting remote buckets and sync
- Supporting automatic update checks
- Supporting package aliases or command-name conflict resolution across packages

## Recommended First Milestone

Deliver a working end-to-end CLI that can:

- Resolve a package name from configured local manifest directories
- Download and install one exact version of a CLI tool for the current platform
- Keep multiple installed versions in the local package store
- Switch the active version
- List installed versions and the active one
- Uninstall non-active versions safely
