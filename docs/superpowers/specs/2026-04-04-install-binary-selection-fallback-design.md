# Hive Install Binary Selection Fallback Design

## Summary

Add an interactive install-time fallback for manifests that do not declare binaries for the current platform. When `hive install <package>` encounters a missing or empty `binaries` list, Hive should download and extract the archive, inspect the installed tree for executable candidates, prompt the user to select one or more binaries, persist those relative paths back into the manifest, and then continue the install using the existing validation and activation flow.

This keeps GitHub sync focused on release metadata while solving the real user experience problem: users often do not know which binaries a package exports or where those binaries live inside the archive.

## Goals

- Let `hive install` recover from manifests that omit `binaries` for the current platform.
- Show users a concrete list of executable candidates discovered from the extracted archive.
- Allow selecting multiple binaries to export.
- Persist the selected relative paths into the manifest so later installs stay non-interactive.
- Reuse the existing install validation, state update, and activation flow after selection.

## Non-Goals

- No change to `hive sync` asset-selection responsibilities in this change.
- No attempt to infer which executables are the "right" ones without user confirmation.
- No new machine-local state file for remembered binary selections.
- No support for selecting non-executable files as exported binaries.
- No automatic selection of all executables by default.

## Problem Statement

Hive currently requires each platform artifact to declare `binaries` up front. That works when the manifest author already knows the archive layout, but breaks down for real packages where users do not know:

- which files inside the archive are actual CLI entrypoints
- whether the binary lives at the archive root or under a subdirectory such as `bin/`
- whether the package exports one binary or several

Requiring users to manually inspect the archive before writing a correct manifest defeats the point of a guided install workflow.

## User Experience

### Normal Install

If the manifest already contains a non-empty `binaries` list for the current platform, `hive install` should behave exactly as it does today. No new prompts appear.

### Missing-Binaries Fallback

If the current platform artifact has no `binaries` field or an empty list, `hive install` should:

1. Download the archive as usual.
2. Extract it into the version directory as usual.
3. Scan the extracted tree for executable candidates.
4. Print a numbered list of candidate relative paths.
5. Prompt the user to select one or more binaries.
6. Persist the selected relative paths into the manifest's current platform artifact entry.
7. Validate those selected paths using the existing binary validation logic.
8. Continue with state updates, activation, and shim generation.

### Follow-Up Installs

Once the manifest has been updated, later installs should not prompt again unless the manifest is changed back to a missing or empty `binaries` list.

Inference:
Persisting the selected binaries into the manifest keeps Hive's package definition authoritative and avoids drift between machines.

## Architecture

### Boundaries

This feature should remain install-driven.

- `src/app.rs` keeps install orchestration and decides when the fallback is needed.
- `src/installer.rs` should expose a focused helper for enumerating executable candidates within an extracted install tree.
- `src/manifest.rs` continues to own manifest parsing and serialization.
- A small prompt trait should encapsulate binary-selection I/O so CLI wiring and tests stay separate.

`hive sync` should not inspect archive contents in this change. It may continue to manage release asset selection independently.

### Manifest Ownership

The manifest file remains the only persisted source of truth for exported binaries. Hive should update only the current platform's `binaries` field and leave:

- package name
- version
- source metadata
- other platform entries

unchanged.

## Candidate Discovery

Hive should inspect the extracted install tree and produce a stable list of candidate binaries.

Rules:

- Include regular files within the install tree that have at least one execute bit set on Unix-like systems.
- Ignore directories.
- Ignore files outside the install tree.
- Return relative paths from the install root, such as `rg` or `bin/nvim`.
- Sort the results lexicographically for deterministic prompts and tests.
- Allow multiple selected results.

Inference:
Using execute-bit detection is conservative enough for Hive's current Linux and macOS scope, and it matches user expectations better than filename heuristics.

## Prompt Behavior

Hive should present a numbered multi-select prompt when fallback is triggered.

Prompt requirements:

- Show all discovered candidate paths in stable order.
- Accept selecting multiple binaries.
- Reject empty selections.
- Return relative paths, not absolute filesystem paths.

The exact terminal UX can follow the current project's simplest prompting style as long as the prompt remains scriptable in tests.

## Data Flow

For each `hive install <package>` invocation:

1. Resolve and load the manifest.
2. Resolve the current platform artifact.
3. Download the artifact bytes and extract them into the install directory.
4. If `binaries` is present and non-empty, continue with the existing validation path.
5. Otherwise, enumerate executable candidates from the extracted tree.
6. Fail if no candidates are found.
7. Prompt the user to choose one or more binaries.
8. Update the in-memory manifest with the selected paths for the current platform.
9. Write the manifest back to disk.
10. Validate that the selected binaries exist within the install tree.
11. Continue with activation and state persistence.

Manifest write-back must happen before Hive claims success for the install, so the package definition and installed result do not diverge.

## Error Handling

Hive must fail closed and avoid partially finalized installs when:

- the current platform artifact is missing
- archive download fails
- extraction fails
- candidate discovery finds no executables
- the user provides an empty or invalid selection
- the manifest update cannot be serialized or written
- selected binary paths fail the existing validation check

If manifest write-back fails after extraction, Hive should report the failure clearly and stop before activation. The version directory may exist on disk, but Hive must not mark it active or update state.

If Hive is later given a non-interactive install mode, this fallback should fail with a clear error instead of blocking on a prompt. That mode is not introduced in this change, but the implementation should keep the prompt boundary explicit so the behavior is easy to add.

## Testing

### Installer-Oriented Tests

Add tests covering:

- candidate discovery returns executable relative paths in stable order
- non-executable files are ignored
- nested executables such as `bin/tool` are discovered

### Install Flow Tests

Add tests covering:

- install with missing `binaries` prompts, persists the selected path, and succeeds
- install with missing `binaries` supports selecting multiple binaries
- install fails clearly when no executable candidates are found
- install fails clearly when the prompt selection is empty
- install does not prompt when `binaries` is already present
- a subsequent install after manifest persistence remains non-interactive
- other platform entries in the manifest are preserved after write-back

The cleanest implementation is to hide prompt I/O behind a small interface so integration tests can provide scripted selections without real terminal input.

## CLI And Documentation

Update CLI help and README install documentation to mention:

- `hive install` may prompt when a manifest omits binaries for the current platform
- Hive discovers executable candidates from the extracted archive
- selected binaries are saved back into the manifest

The README should include a concise example showing a manifest before and after install fills in the `binaries` list.

## Open Decisions Closed In This Spec

- The fallback happens during `install`, not `sync`.
- Multiple binaries can be selected.
- Selected binaries are persisted into the manifest, not local state.
- Candidate discovery is based on executable files in the extracted tree.
- Existing manifests with declared binaries remain non-interactive.
