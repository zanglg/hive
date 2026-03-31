# Hive Proxy Environment Support Design

## Summary

Add proxy support for HTTP(S) artifact downloads in Hive v1. Proxy configuration is environment-only. Hive-specific environment variables override standard proxy environment variables. Proxy authentication is supported only through credentials embedded in the proxy URL.

## Goals

- Support authenticated and unauthenticated proxies for HTTP(S) downloads.
- Allow per-process override through environment variables without changing manifests or install state.
- Keep proxy resolution isolated from install, extraction, activation, and rollback logic.
- Preserve existing `file://` download behavior.

## Non-Goals

- No config-file-based proxy settings.
- No separate Hive proxy username or password variables.
- No proxy support for operations that do not perform HTTP(S) requests.
- No retries, timeout tuning, or broader downloader refactor in this change.

## Environment Variables

Hive reads these Hive-specific variables first:

- `HIVE_HTTP_PROXY`
- `HIVE_HTTPS_PROXY`
- `HIVE_ALL_PROXY`
- `HIVE_NO_PROXY`

If a Hive-specific variable for a slot is unset, Hive falls back to the standard variables for that same slot:

- `HTTP_PROXY` / `http_proxy`
- `HTTPS_PROXY` / `https_proxy`
- `ALL_PROXY` / `all_proxy`
- `NO_PROXY` / `no_proxy`

Inference:
Hive-specific variables should be uppercase-only. Standard variables should support both uppercase and lowercase because that is the de facto ecosystem convention.

## Precedence Rules

For each slot, the effective value is resolved independently:

1. `HIVE_*`
2. Standard uppercase variable
3. Standard lowercase variable

Examples:

- If both `HIVE_HTTP_PROXY` and `HTTP_PROXY` are set, Hive uses `HIVE_HTTP_PROXY`.
- If `HIVE_HTTPS_PROXY` is unset and `HTTPS_PROXY` is set, Hive uses `HTTPS_PROXY`.
- If `NO_PROXY` is unset and `no_proxy` is set, Hive uses `no_proxy`.

## Authentication Rules

Proxy authentication is supported only when credentials are embedded in the proxy URL, for example:

- `http://user:pass@proxy.internal:8080`

Hive must not support separate username or password environment variables.

If the proxy URL contains credentials, Hive passes the URL through to the HTTP client. Error messages must not echo the full URL because it may contain secrets.

## Architecture

Add a small proxy-focused module, for example `src/proxy.rs`, with two responsibilities:

1. Resolve effective proxy settings from the environment into a small internal struct.
2. Build a configured `reqwest::blocking::Client` from those settings.

The install path in `src/app.rs` should stop calling `reqwest::blocking::get` directly. Instead, it should construct the HTTP client once per `hive install` invocation and use that client for HTTP(S) downloads.

`file://` downloads remain direct filesystem copies and must not go through the HTTP client.

## Proposed Internal Model

Use a narrow struct that keeps transport policy separate from install logic:

```rust
struct ProxySettings {
    http_proxy: Option<ResolvedProxy>,
    https_proxy: Option<ResolvedProxy>,
    all_proxy: Option<ResolvedProxy>,
    no_proxy: Option<String>,
}

struct ResolvedProxy {
    env_name: &'static str,
    url: String,
}
```

`env_name` is retained so validation errors can identify the bad variable without printing secrets.

## Client Construction

The client builder should apply settings in this order:

1. Apply `http_proxy` with `reqwest::Proxy::http(...)` when present.
2. Apply `https_proxy` with `reqwest::Proxy::https(...)` when present.
3. Apply `all_proxy` as the fallback proxy when present.
4. Apply `no_proxy` only if the current `reqwest` version supports it correctly for blocking clients.

If the library cannot support `no_proxy` correctly in the existing dependency version, Hive should fail clearly when `HIVE_NO_PROXY`, `NO_PROXY`, or `no_proxy` is set, rather than silently ignoring the variable.

## Data Flow

For `hive install <package>`:

1. Resolve the manifest and chosen artifact as today.
2. Resolve proxy environment variables once.
3. Build one blocking HTTP client.
4. Download the artifact with that client when the URL is `http://` or `https://`.
5. Continue with checksum verification, extraction, normalization, activation, and rollback exactly as today.

This keeps proxy behavior at the network boundary and out of package state transitions.

## Error Handling

Validation should fail early during client construction.

Cases:

- Malformed proxy URL:
  return an error that names the environment variable, for example `invalid proxy URL in HIVE_HTTPS_PROXY`.
- Unsupported `no_proxy` support:
  return an error that names the environment variable and states that `no_proxy` is not supported yet.
- HTTP request failure:
  preserve the current download failure behavior after the client has been built.

Error messages must not include credential-bearing URLs verbatim.

## Testing

### Unit Tests

Add focused tests for the proxy resolver:

- `HIVE_*` overrides standard variables.
- Standard uppercase overrides standard lowercase.
- Per-scheme values resolve independently.
- `ALL_PROXY` is used when scheme-specific values are absent.
- Malformed URLs produce an error that includes the environment variable name.
- Embedded credentials are preserved in client configuration input and never echoed in errors.
- `NO_PROXY` behavior is either implemented and verified or rejected and verified.

### Integration Tests

Add light integration coverage around the download path:

- `file://` downloads still bypass HTTP client/proxy logic.
- HTTP(S) downloads construct the client through the proxy module.

The tests should not depend on a live proxy server. The main value is verifying resolution, validation, and wiring boundaries.

## Implementation Notes

- Keep the proxy module independent from `HivePaths`.
- Avoid reading environment variables directly in the download function after the new module exists.
- Reuse one client per install command rather than rebuilding it for each request.
- Update the README with the supported environment variables and the rule that authentication must be embedded in the proxy URL.

## Open Decision Closed In This Spec

The following choices are intentionally fixed here to avoid ambiguity during implementation:

- Proxy settings are environment-only.
- Hive-specific variables override standard variables.
- Authentication is URL-only.
- Separate Hive auth variables are not supported.
- Standard lowercase proxy variables are supported as fallback aliases.
- Unsupported `no_proxy` behavior must fail explicitly rather than being ignored.
