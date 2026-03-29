# Hive Install Layout Normalization Design

## Summary

Extend `hive-v1` so package manifests describe binaries relative to a normalized installed layout instead of the raw archive root. Activation switches through a package-local `current` symlink, and archive support expands to include `tar.xz`.

This makes the currently omitted tools manifestable:

- `bat`
- `codex`
- `fd`
- `gh`
- `helix`
- `neovim`
- `ripgrep`
- `television`
- `uv`

## Goals

- Keep the package store layout rooted at `pkgs/<package>/<version>/`
- Normalize extracted archives by removing one redundant top-level wrapper directory
- Interpret manifest `binaries` as paths relative to the normalized installed root
- Derive shim names from the basename of each declared binary path
- Make `use` switch a package-level `current` symlink instead of rebuilding shims
- Add `tar.xz` support for upstream releases such as Helix
- Preserve compatibility with existing simple manifests such as `binaries = ["rg"]`

## Non-Goals

- Explicit export renaming in manifests
- Remote manifest discovery
- Automatic fallback to another installed version when the active version is removed
- A new CLI for resolving non-package binary names

## Current Problems

The current implementation assumes each manifest binary entry serves two unrelated roles:

- the extracted file path
- the shim name in `~/.local/bin/hive`

That breaks common upstream release layouts:

- archives with a wrapper directory such as `gh_2.89.0_linux_amd64/bin/gh`
- binaries renamed by target triple such as `codex-x86_64-apple-darwin`
- archives packaged as `tar.xz` such as Helix

It also makes `use` heavier than necessary because switching versions rewrites every shim directly to version-specific targets.

## Proposed Layout

### Package Store

Installed package contents remain under:

- `~/.local/share/hive/pkgs/<package>/<version>/`

Each package also gets an activation symlink:

- `~/.local/share/hive/pkgs/<package>/current -> <version>`

`current` always points to one installed version directory for that package or does not exist when no version is active.

### Shim Directory

Hive-managed shims remain under:

- `~/.local/bin/hive/`

Each shim points to the package `current` tree, not directly to a concrete version.

Example:

- manifest `binaries = ["bin/hx"]`
- shim `~/.local/bin/hive/hx -> ~/.local/share/hive/pkgs/helix/current/bin/hx`

`use helix 25.07.1` only updates `pkgs/helix/current`.

## Archive Normalization

After extraction, Hive normalizes the installed tree before validating binaries.

Rule:

- if the extracted archive root contains exactly one entry and that entry is a directory, move that directory's contents up into the version install root
- otherwise keep the extracted tree as-is

Examples:

- `gh_2.89.0_linux_amd64/bin/gh` becomes installed as `bin/gh`
- `bat-v0.26.1-x86_64-apple-darwin/bat` becomes installed as `bat`
- flat archives such as `opencode` remain unchanged

This rule is intentionally narrow. Hive strips one wrapper directory and does not attempt more aggressive reshaping.

## Manifest Semantics

`binaries` remains a list of strings, but its meaning changes to:

- paths relative to the normalized installed root

Shim names are derived from the basename of each declared path.

Examples:

- `binaries = ["bin/gh"]` exports shim `gh`
- `binaries = ["bin/hx"]` exports shim `hx`
- `binaries = ["uv", "uvx"]` exports shims `uv` and `uvx`
- `binaries = ["codex"]` exports shim `codex`

Existing manifests such as `binaries = ["rg"]` continue to work without modification.

## Command Behavior

### install

- Download the selected archive
- Extract into a staging directory
- Normalize the extracted tree by stripping one wrapper directory when present
- Move the normalized tree into `pkgs/<package>/<version>/`
- Validate each declared binary path against the normalized install root
- If this is the first installed version for the package:
  - create `pkgs/<package>/current -> <version>`
  - create package shims pointing into `current`
- Otherwise leave the active version unchanged

### use

- Validate the requested version is installed
- Validate each declared binary path exists under `pkgs/<package>/<version>/`
- Update `pkgs/<package>/current` to point to the requested version
- Ensure shims exist for the package and point into `current`

This keeps shim targets stable across version changes.

### uninstall

- Remove the version directory
- If the removed version was inactive:
  - leave `current` and shims untouched
- If the removed version was active and `--force` was used:
  - remove `pkgs/<package>/current`
  - remove all shims for that package
  - do not automatically activate another installed version

This matches current conservative behavior while fitting the new activation model.

### which

`which <package>` can no longer assume the package name matches the exported binary name.

For `hive-v1`, behavior becomes:

- load the manifest for the package on the current platform
- if it declares exactly one binary, resolve that binary through `current`
- if it declares more than one binary, return a clear error explaining that `which <package>` is ambiguous for multi-binary packages in v1

This preserves a simple CLI while avoiding wrong answers for packages such as `uv`.

## Archive Support

Supported archive kinds become:

- `tar.gz`
- `tar.xz`
- `zip`

`tar.xz` extraction should use the standard Rust xz decoder path and follow the same staging and normalization flow as `tar.gz`.

## Testing

Add tests for:

- manifest binary validation against normalized paths
- wrapper-directory stripping during install
- flat archives that must remain unchanged
- shim naming derived from binary basenames
- `use` updating `current` instead of version-specific shim targets
- forced uninstall removing `current` and package shims
- `which` behavior for single-binary and multi-binary packages
- successful installation of a `tar.xz` archive

## Risks

- Archive normalization must not accidentally discard sibling files
- `current` updates must remain atomic enough to avoid broken shims during switches
- Multi-binary package handling in `which` is intentionally limited and may need a CLI extension later

## Implementation Notes

- Keep state store semantics unchanged where possible; activation lives in filesystem links
- Update the README manifest example and limits section after the code change lands
- Existing installed packages may need reinstallation if they were installed under the old raw archive layout assumptions
