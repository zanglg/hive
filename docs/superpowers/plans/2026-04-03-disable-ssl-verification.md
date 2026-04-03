# Disable SSL Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `HIVE_INSECURE_SSL` so Hive can disable TLS certificate verification for all HTTPS requests made through the shared blocking HTTP client.

**Architecture:** Keep transport policy centralized in `src/proxy.rs`. Extend the proxy module to parse a Hive-specific insecure-TLS env var, apply it when building the shared `reqwest::blocking::Client`, and verify the behavior through transport-layer unit tests plus CLI-facing regression tests. No install, sync, or manifest code should parse the env var directly.

**Tech Stack:** Rust, `reqwest::blocking`, `cargo test`

---

## File Structure

- Modify: `src/proxy.rs`
  Responsibility: Parse `HIVE_INSECURE_SSL`, carry the result in transport settings, and apply `danger_accept_invalid_certs(true)` when building the shared HTTP client.
- Modify: `tests/cli_install.rs`
  Responsibility: Cover install-path wiring and failure behavior for invalid `HIVE_INSECURE_SSL` values.
- Modify: `tests/cli_sync.rs`
  Responsibility: Cover sync-path wiring and failure behavior for invalid `HIVE_INSECURE_SSL` values.
- Modify: `README.md`
  Responsibility: Document the new environment variable next to the existing proxy transport configuration.

### Task 1: Add Transport-Layer Parsing And Unit Tests

**Files:**
- Modify: `src/proxy.rs`
- Test: `src/proxy.rs`

- [ ] **Step 1: Write the failing transport tests**

Add these tests to the existing `#[cfg(test)]` module in `src/proxy.rs`:

```rust
    #[test]
    fn insecure_ssl_defaults_to_disabled() {
        let settings = resolve_transport_settings_from(|_| None).unwrap();

        assert!(!settings.insecure_ssl);
    }

    #[test]
    fn insecure_ssl_accepts_truthy_values_case_insensitively() {
        for value in ["1", "true", "TRUE", "Yes", "on"] {
            let env = HashMap::from([("HIVE_INSECURE_SSL", value)]);

            let settings =
                resolve_transport_settings_from(|name| env.get(name).map(|value| value.to_string()))
                    .unwrap();

            assert!(settings.insecure_ssl, "expected `{value}` to enable insecure SSL");
        }
    }

    #[test]
    fn insecure_ssl_rejects_invalid_values() {
        let env = HashMap::from([("HIVE_INSECURE_SSL", "maybe")]);

        let error =
            resolve_transport_settings_from(|name| env.get(name).map(|value| value.to_string()))
                .unwrap_err();

        assert!(error.contains("HIVE_INSECURE_SSL"));
        assert!(error.contains("invalid boolean value"));
    }
```

- [ ] **Step 2: Run the proxy tests to verify they fail**

Run: `cargo test proxy::tests::insecure_ssl_defaults_to_disabled proxy::tests::insecure_ssl_accepts_truthy_values_case_insensitively proxy::tests::insecure_ssl_rejects_invalid_values`

Expected: FAIL with missing `resolve_transport_settings_from` or missing `insecure_ssl` members.

- [ ] **Step 3: Implement minimal transport settings parsing**

Refactor `src/proxy.rs` so proxy resolution and the insecure-TLS toggle are returned together, then build the client from that struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TransportSettings {
    http_proxy: Option<ResolvedValue>,
    https_proxy: Option<ResolvedValue>,
    all_proxy: Option<ResolvedValue>,
    no_proxy: Option<ResolvedValue>,
    insecure_ssl: bool,
}

pub fn build_http_client() -> Result<reqwest::blocking::Client, String> {
    let settings = resolve_transport_settings_from(|name| std::env::var(name).ok())?;
    let https_proxy = effective_https_proxy(&settings);
    let no_proxy = settings
        .no_proxy
        .as_ref()
        .and_then(|resolved| reqwest::NoProxy::from_string(&resolved.value));
    let mut builder = reqwest::blocking::Client::builder().no_proxy();

    if settings.insecure_ssl {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(proxy) = settings.http_proxy.clone() {
        builder = builder.proxy(
            reqwest::Proxy::http(&proxy.value)
                .map_err(|_| format!("invalid proxy URL in {}", proxy.env_name))?
                .no_proxy(no_proxy.clone()),
        );
    }

    if let Some(proxy) = https_proxy {
        builder = builder.proxy(
            reqwest::Proxy::https(&proxy.value)
                .map_err(|_| format!("invalid proxy URL in {}", proxy.env_name))?
                .no_proxy(no_proxy.clone()),
        );
    }

    if let Some(proxy) = settings.all_proxy.clone() {
        builder = builder.proxy(
            reqwest::Proxy::all(&proxy.value)
                .map_err(|_| format!("invalid proxy URL in {}", proxy.env_name))?
                .no_proxy(no_proxy),
        );
    }

    builder
        .build()
        .map_err(|error| format!("failed to build HTTP client: {error}"))
}

fn resolve_transport_settings_from<F>(mut get: F) -> Result<TransportSettings, String>
where
    F: FnMut(&str) -> Option<String>,
{
    Ok(TransportSettings {
        http_proxy: resolve_value(&mut get, "HIVE_HTTP_PROXY", "HTTP_PROXY", "http_proxy"),
        https_proxy: resolve_value(&mut get, "HIVE_HTTPS_PROXY", "HTTPS_PROXY", "https_proxy"),
        all_proxy: resolve_value(&mut get, "HIVE_ALL_PROXY", "ALL_PROXY", "all_proxy"),
        no_proxy: resolve_value(&mut get, "HIVE_NO_PROXY", "NO_PROXY", "no_proxy"),
        insecure_ssl: resolve_insecure_ssl(&mut get)?,
    })
}

fn resolve_insecure_ssl<F>(get: &mut F) -> Result<bool, String>
where
    F: FnMut(&str) -> Option<String>,
{
    let Some(value) = get("HIVE_INSECURE_SSL") else {
        return Ok(false);
    };

    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        _ => Err("invalid boolean value in HIVE_INSECURE_SSL".to_string()),
    }
}
```

Also update existing test imports in `src/proxy.rs` from `ProxySettings` to `TransportSettings` and from `resolve_proxy_settings_from` to `resolve_transport_settings_from`.

- [ ] **Step 4: Run the proxy tests to verify they pass**

Run: `cargo test proxy::tests`

Expected: PASS for existing proxy tests and the new insecure-SSL parsing tests.

- [ ] **Step 5: Commit the transport-layer change**

```bash
git add src/proxy.rs
git commit -m "feat: support insecure SSL transport setting"
```

### Task 2: Add Install And Sync Regression Tests For Invalid Values

**Files:**
- Modify: `tests/cli_install.rs`
- Modify: `tests/cli_sync.rs`
- Test: `tests/cli_install.rs`
- Test: `tests/cli_sync.rs`

- [ ] **Step 1: Write the failing install-path regression test**

Add this test to `tests/cli_install.rs` near the existing invalid-proxy transport test:

```rust
#[test]
fn install_rejects_invalid_hive_insecure_ssl_value_for_https_downloads() {
    let _env = tests_support::lock_env();
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let archive_name = "rg.tar.gz";
    let archive_path = tests_support::write_named_tar_gz(temp.path(), archive_name, "rg");
    let archive_bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(&archive_bytes));

    tests_support::write_manifest_with_binaries_with_archive(
        &paths,
        "rg",
        "14.1.0",
        &archive_path,
        &checksum,
        &["rg"],
        "tar.gz",
    );
    let manifest_path = paths.manifest_dirs[0].join("rg.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .unwrap()
        .replace(
            &format!("file://{}", archive_path.display()),
            "https://example.invalid/rg.tar.gz",
        );
    fs::write(&manifest_path, manifest).unwrap();

    unsafe {
        std::env::set_var("HIVE_INSECURE_SSL", "maybe");
    }
    let error = app::run_capture(
        Cli::try_parse_from(["hive", "install", "rg"]).unwrap(),
        paths,
    )
    .unwrap_err();
    unsafe {
        std::env::remove_var("HIVE_INSECURE_SSL");
    }

    assert!(error.contains("HIVE_INSECURE_SSL"));
    assert!(error.contains("invalid boolean value"));
}
```

- [ ] **Step 2: Write the failing sync-path regression test**

Add this test to `tests/cli_sync.rs` near `sync_rejects_invalid_hive_http_proxy_for_github_requests`:

```rust
#[test]
fn sync_rejects_invalid_hive_insecure_ssl_value_for_github_requests() {
    let _env = tests_support::lock_env();
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());

    unsafe {
        std::env::set_var("HIVE_INSECURE_SSL", "maybe");
    }
    let error = sync::sync_repo_with_api_base(
        &paths,
        "BurntSushi/ripgrep",
        "https://api.github.com",
    )
    .unwrap_err();
    unsafe {
        std::env::remove_var("HIVE_INSECURE_SSL");
    }

    assert!(error.contains("HIVE_INSECURE_SSL"));
    assert!(error.contains("invalid boolean value"));
}
```

- [ ] **Step 3: Run the new CLI tests to verify they fail first**

Run: `cargo test install_rejects_invalid_hive_insecure_ssl_value_for_https_downloads`

Expected: FAIL before the transport parser is wired into client construction, or PASS only if Task 1 is already complete.

Run: `cargo test sync_rejects_invalid_hive_insecure_ssl_value_for_github_requests`

Expected: FAIL before the transport parser is wired into client construction, or PASS only if Task 1 is already complete.

If Task 1 is already complete, use these runs as regression proof that the new CLI coverage is active.

- [ ] **Step 4: Adjust the tests to match the final transport behavior if needed**

If Task 1 is already implemented when adding these tests, keep only the assertions above and do not add any extra mocking. The point of these tests is that both flows fail before making network requests because `proxy::build_http_client()` validates `HIVE_INSECURE_SSL` centrally.

No additional production code should be required in this task if Task 1 is complete.

- [ ] **Step 5: Run the focused CLI transport regressions**

Run: `cargo test install_rejects_invalid_hive_insecure_ssl_value_for_https_downloads`

Expected: PASS with the install command returning an error that names `HIVE_INSECURE_SSL`.

Run: `cargo test sync_rejects_invalid_hive_insecure_ssl_value_for_github_requests`

Expected: PASS with the sync command returning an error that names `HIVE_INSECURE_SSL`.

- [ ] **Step 6: Commit the CLI regression tests**

```bash
git add tests/cli_install.rs tests/cli_sync.rs
git commit -m "test: cover insecure SSL env validation"
```

### Task 3: Document The New Transport Setting And Run Full Verification

**Files:**
- Modify: `README.md`
- Test: `src/proxy.rs`
- Test: `tests/cli_install.rs`
- Test: `tests/cli_sync.rs`

- [ ] **Step 1: Write the README update**

Add this bullet to the `## Proxy Environment` section in `README.md` after the proxy variable list:

```md
- `HIVE_INSECURE_SSL=1` disables TLS certificate verification for all HTTPS requests made by the current Hive process. Use this only in controlled environments such as local testing or corporate interception proxy setups.
```

- [ ] **Step 2: Run the targeted verification commands**

Run: `cargo test proxy::tests`

Expected: PASS

Run: `cargo test install_rejects_invalid_hive_insecure_ssl_value_for_https_downloads`

Expected: PASS

Run: `cargo test sync_rejects_invalid_hive_insecure_ssl_value_for_github_requests`

Expected: PASS

- [ ] **Step 3: Run the broader regression suite for touched behavior**

Run: `cargo test --test cli_install`

Expected: PASS

Run: `cargo test --test cli_sync`

Expected: PASS

- [ ] **Step 4: Commit the documentation and verification-backed finish**

```bash
git add README.md
git commit -m "docs: document insecure SSL env support"
```

## Self-Review

- Spec coverage:
  Task 1 covers centralized environment parsing, strict validation, global client behavior, and transport-layer unit tests from the spec.
  Task 2 covers sync/install wiring at the shared client boundary and proves invalid values fail before outbound network requests.
  Task 3 covers README documentation and final verification.
- Placeholder scan:
  No `TODO`, `TBD`, or implicit "handle appropriately" steps remain; every code-changing step includes concrete code or exact text.
- Type consistency:
  The plan consistently uses `TransportSettings`, `resolve_transport_settings_from`, `resolve_insecure_ssl`, and `insecure_ssl`.
