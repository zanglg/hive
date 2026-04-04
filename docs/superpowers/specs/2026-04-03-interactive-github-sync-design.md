# Hive Interactive GitHub Sync Design

## Summary

Change `hive sync <owner>/<repo>` from automatic asset mapping to an interactive, current-platform-only workflow. On every sync run, Hive fetches the latest qualifying GitHub release, shows the release assets, and prompts the user to choose the asset for the current platform. Hive then prompts for the binary path or paths inside that archive, computes the checksum, and writes both the install artifact data and the saved GitHub sync selection into the manifest.

This fixes repositories such as `neovim/neovim`, where release asset names do not fit Hive's current hard-coded filename matcher.

## Goals

- Make `hive sync neovim/neovim` and similar repositories work without relying on narrow filename conventions.
- Always ask the user to select an asset for the current platform during `hive sync`.
- Persist the user's exact selected asset filename and binary paths in the manifest.
- Regenerate the current platform's manifest entry from the latest release on each sync run.
- Keep `hive install` unchanged by continuing to read only `[platform.*]` install data.

## Non-Goals

- No attempt to sync all supported platforms in one run.
- No regex, glob, or substring matching for saved asset selections in this change.
- No separate state file for GitHub sync choices.
- No automatic archive inspection to discover binaries.
- No support for non-interactive `hive sync` in this change.

## Problem Statement

The current sync flow assumes GitHub release assets can be mapped to Hive platforms by hard-coded filename fragments such as `x86_64-unknown-linux` and `aarch64-apple-darwin`. That works for a narrow set of repositories but fails for widely used projects with different naming conventions.

The current manifest schema also lacks any persisted sync-selection metadata, so Hive has no place to store user intent about which release asset corresponds to the current platform or which binaries should be exported after install.

## User Experience

### First Sync

When the user runs `hive sync neovim/neovim` on the current platform, Hive should:

1. Fetch the latest qualifying GitHub release for the configured channel.
2. Print the release tag and a numbered list of release asset filenames.
3. Prompt the user to choose one asset for the current platform.
4. Infer the archive kind from the selected filename and reject unsupported suffixes before any manifest write.
5. Prompt for one or more binary paths relative to the extracted archive root.
6. Download the selected asset, compute its checksum, and write a manifest containing:
   - package name and version
   - GitHub source metadata
   - saved sync selection for the current platform
   - `[platform.<current-platform>]` artifact data

### Later Syncs

`hive sync` remains interactive on every run, but only for the current platform.

If the manifest already contains a saved selection for the current platform, Hive should use that saved asset filename and binary list as the prompt defaults. The user can keep the saved values or choose a different asset and different binaries. Hive then rewrites only the current platform's artifact entry and its corresponding saved sync-selection metadata.

This keeps the command explicitly user-driven on every sync while still preserving prior choices in the manifest.

## Manifest Format

Hive should extend the GitHub source configuration to store per-platform sync selections alongside the existing `repo` and `channel` fields.

Conceptual shape:

```toml
name = "neovim"
version = "0.11.0"

[source.github]
repo = "neovim/neovim"
channel = "stable"

[source.github.platform.macos-aarch64]
asset = "nvim-macos-arm64.tar.gz"
binaries = ["bin/nvim"]

[platform.macos-aarch64]
url = "https://github.com/neovim/neovim/releases/download/v0.11.0/nvim-macos-arm64.tar.gz"
checksum = "sha256:..."
archive = "tar.gz"
binaries = ["bin/nvim"]
```

The exact TOML layout may vary slightly to match `serde` ergonomics, but the schema must preserve these semantics:

- GitHub source config includes per-platform saved asset filename and binary paths.
- Installable artifact data remains under `[platform.*]`.
- Sync metadata and install metadata live in the same manifest file.

Inference:
Storing sync-selection data in the manifest is the best fit for Hive's current architecture because manifests are already the user-visible source of truth for package definitions.

## Architecture

### Boundary

`src/sync.rs` should remain the main orchestration layer for GitHub manifest sync, but its responsibilities need to shift:

- release selection stays in the GitHub client
- current-platform detection stays in `src/platform.rs`
- sync owns the interactive prompt flow and manifest regeneration
- manifest types own serialization of the new GitHub sync-selection metadata

`hive install` should not need to understand any of the new sync metadata. It continues to resolve the current platform from `[platform.*]` and ignores `[source.github.*]` beyond normal deserialization.

### Current Platform Only

Sync operates on exactly one Hive platform per run: the platform returned by `Platform::current()`.

If the manifest already contains artifact entries or saved sync selections for other platforms, sync must preserve them unchanged. The command updates only:

- the manifest version
- the saved GitHub sync selection for the current platform
- the `[platform.<current-platform>]` artifact entry

This keeps the feature narrowly scoped and avoids prompting for platforms the current machine cannot validate interactively.

## Sync Flow

For each `hive sync <owner>/<repo>` invocation:

1. Parse the repository name and resolve the manifest path.
2. Load any existing manifest and GitHub source metadata.
3. Resolve the GitHub channel from existing manifest data or default to `stable`.
4. Detect the current Hive platform.
5. Fetch the latest qualifying GitHub release and its assets.
6. Fail if the release has no assets.
7. Render a numbered asset list in the terminal.
8. Prompt for the current platform's asset selection.
9. Prompt for binary paths, using any saved paths as defaults when present.
10. Infer the archive kind from the chosen filename.
11. Download the selected asset and compute its checksum.
12. Build a new manifest by preserving unrelated data and replacing only the current platform's sync metadata and artifact entry.
13. Write the manifest only after the full operation succeeds.

## Error Handling

Hive must fail closed and leave the existing manifest unchanged when:

- the repository string is invalid
- GitHub release metadata cannot be fetched
- the latest qualifying release has no assets
- the user selects an invalid asset index
- the selected asset filename has an unsupported archive suffix
- binary input is empty or invalid
- the selected asset cannot be downloaded
- checksum calculation fails
- manifest serialization or write fails

If the manifest already contains a saved asset filename for the current platform but the latest release no longer includes it, Hive should still show the latest asset list and require the user to choose a new asset. It must not silently guess a replacement.

## Testing

### Manifest Tests

Add or update manifest serialization tests to cover:

- GitHub source metadata with per-platform saved asset selection
- round-trip TOML parsing and serialization for the new schema
- backward compatibility for older manifests that have `repo` and `channel` only

### Sync Tests

Add integration-focused sync tests covering:

- first interactive sync writes current-platform sync metadata and artifact data
- later sync reuses saved defaults but still accepts user confirmation or replacement
- unsupported archive suffix is rejected without mutating the manifest
- a missing previously saved filename forces explicit reselection
- sync preserves artifact entries for other platforms already present in the manifest
- repos with nonstandard asset names such as a Neovim-like fixture can be synced successfully

The cleanest way to test this is to separate prompt handling behind a small interface so tests can provide scripted answers without depending on real terminal input.

## CLI And Documentation

Update command help and the README sync section to reflect the new workflow:

- `hive sync` is interactive
- it prompts only for the current platform
- it saves the chosen asset filename and binary paths into the manifest
- future syncs remain interactive but prefill previously saved values

The README example should include a manifest snippet that shows both GitHub sync metadata and the generated current-platform artifact entry.

## Open Decisions Closed In This Spec

- `hive sync` is interactive on every run.
- Sync prompts only for the current platform.
- The user chooses from actual release asset filenames rather than Hive guessing by convention.
- Saved selections use exact asset filenames, not patterns.
- Binary paths are provided by the user and saved in the manifest.
- Sync metadata is stored in the manifest, not in a separate state file.
- `hive install` behavior remains unchanged.
