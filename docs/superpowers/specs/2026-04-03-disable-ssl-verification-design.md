# Hive Disable SSL Verification Design

## Summary

Add support for disabling TLS certificate verification for all HTTPS requests made by Hive v1 through a Hive-specific environment variable. The setting is process-scoped, environment-only, and applied when constructing the shared blocking HTTP client.

## Goals

- Support controlled environments where HTTPS inspection or self-signed certificates would otherwise block Hive network requests.
- Keep TLS verification policy at the transport boundary rather than spreading it across install and sync flows.
- Apply one consistent behavior to all HTTPS requests made through Hive's shared HTTP client.
- Preserve secure-by-default behavior when the new environment variable is unset.

## Non-Goals

- No CLI flag for insecure TLS in this change.
- No manifest-level or persisted config for TLS verification.
- No certificate pinning, custom CA bundles, or client certificate support.
- No change to `file://` downloads or any non-HTTP transport.

## Environment Variable

Hive adds one new environment variable:

- `HIVE_INSECURE_SSL`

This variable is Hive-specific only. There is no standard-variable fallback for this behavior.

Accepted truthy values are:

- `1`
- `true`
- `yes`
- `on`

Parsing is case-insensitive. Unset means TLS certificate verification remains enabled.

Any other non-empty value is invalid and must fail client construction with an explicit error that names the environment variable, for example:

- `invalid boolean value in HIVE_INSECURE_SSL`

Inference:
An explicit parse error is better than silently treating typos as disabled or ignored behavior because this setting changes a security property.

## Architecture

The existing transport builder in `src/proxy.rs` remains the single place where Hive derives network policy from environment variables and constructs `reqwest::blocking::Client`.

This module already owns proxy resolution and client creation. The new TLS verification toggle should be added alongside those responsibilities rather than creating a second transport configuration entry point.

The resulting boundary is:

1. Install and sync code ask for a configured HTTP client.
2. The transport layer reads proxy-related variables and `HIVE_INSECURE_SSL`.
3. The transport layer returns one configured blocking client for the current process invocation.

No install, sync, manifest, or state-management logic should parse or reason about `HIVE_INSECURE_SSL` directly.

## Client Construction

When `HIVE_INSECURE_SSL` is enabled, the client builder should call:

```rust
danger_accept_invalid_certs(true)
```

This change should be applied on the same `reqwest::blocking::ClientBuilder` instance that already receives proxy configuration.

Builder order should remain straightforward:

1. Start with the default blocking client builder.
2. Apply `danger_accept_invalid_certs(true)` when `HIVE_INSECURE_SSL` is enabled.
3. Apply proxy and no-proxy settings as currently supported.
4. Build the client once and reuse it for the command.

The ordering is not meant to create separate policy modes; it simply keeps all builder mutations in one place.

## Scope

The setting applies to all HTTPS requests performed through Hive's shared client, including:

- GitHub API requests in `hive sync`
- HTTPS artifact downloads used during install flows

It does not apply to:

- `file://` paths, which bypass HTTP client usage entirely
- Non-HTTPS operations that do not involve TLS certificate validation

## Error Handling

Validation should happen before the client is built.

Cases:

- If `HIVE_INSECURE_SSL` is unset, proceed with normal certificate verification.
- If `HIVE_INSECURE_SSL` contains an accepted truthy value, disable certificate verification for the client.
- If `HIVE_INSECURE_SSL` contains any other non-empty value, return an error naming the variable.
- Existing proxy validation errors remain unchanged.

This feature does not require special runtime error rewriting beyond the invalid-value case because the TLS behavior is configured entirely at client construction time.

## Testing

### Unit Tests

Add transport-layer tests covering:

- Unset `HIVE_INSECURE_SSL` leaves insecure mode disabled.
- Accepted truthy values enable insecure mode.
- Parsing is case-insensitive.
- Invalid values fail with an error naming `HIVE_INSECURE_SSL`.
- Proxy resolution behavior remains unchanged when the new variable is absent.

The simplest shape is to extend the existing test coverage in `src/proxy.rs` with a small parser-focused helper or transport settings struct that can be asserted without needing live network access.

### Integration Tests

Keep integration scope narrow:

- Verify `file://` install behavior remains unaffected.
- Preserve the existing boundary where sync and install obtain network behavior through the shared client builder rather than per-call ad hoc logic.

Live TLS test servers are unnecessary for this change. The main value is verifying policy resolution and wiring.

## Documentation

Update `README.md` in the transport configuration area next to the proxy environment variables.

The documentation should state:

- `HIVE_INSECURE_SSL=1` disables TLS certificate verification for the current Hive process
- The setting affects all HTTPS requests made by Hive
- It should be used only in controlled environments, such as local testing or corporate interception proxy setups

## Open Decisions Closed In This Spec

The following choices are fixed for implementation:

- The feature is environment-only.
- The variable name is `HIVE_INSECURE_SSL`.
- The setting applies globally to all HTTPS requests made by Hive.
- There is no CLI flag in this change.
- Invalid values fail explicitly rather than being ignored.
- `file://` behavior is unchanged.
