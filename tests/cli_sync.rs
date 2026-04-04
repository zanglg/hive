#[path = "support/mod.rs"]
mod tests_support;

use clap::Parser;
use hive::{
    app,
    cli::{Cli, Commands},
    github::GitHubClient,
    manifest::Manifest,
    proxy, sync,
};
use sha2::{Digest, Sha256};
use std::fs;
use tempfile::tempdir;

#[test]
fn parses_sync_command_with_repo_argument() {
    let cli = Cli::try_parse_from(["hive", "sync", "BurntSushi/ripgrep"]).unwrap();

    match cli.command {
        Commands::Sync { repo } => assert_eq!(repo, "BurntSushi/ripgrep"),
        _ => panic!("expected sync command"),
    }
}

#[test]
fn manifest_round_trips_github_platform_selection() {
    let contents = r#"
name = "rg"
version = "14.1.0"

[source.github]
repo = "BurntSushi/ripgrep"
channel = "stable"

[source.github.platform.macos-aarch64]
asset = "ripgrep-14.1.0-aarch64-apple-darwin.tar.gz"
binaries = ["rg"]

[platform.macos-aarch64]
url = "https://example.invalid/rg-14.1.0-aarch64.tar.gz"
checksum = "sha256:2222222222222222222222222222222222222222222222222222222222222222"
archive = "tar.gz"
binaries = ["rg"]
"#;

    let manifest = Manifest::from_toml(contents).unwrap();
    let github = manifest.source.as_ref().unwrap().github.as_ref().unwrap();
    let selection = github.platform.get("macos-aarch64").unwrap();

    assert_eq!(github.repo, "BurntSushi/ripgrep");
    assert_eq!(github.channel, "stable");
    assert_eq!(
        selection.asset,
        "ripgrep-14.1.0-aarch64-apple-darwin.tar.gz"
    );
    assert_eq!(selection.binaries, vec!["rg"]);

    let rendered = manifest.to_toml().unwrap();
    assert!(rendered.contains("[source.github.platform.macos-aarch64]"));
    assert!(rendered.contains("asset = \"ripgrep-14.1.0-aarch64-apple-darwin.tar.gz\""));
}

#[test]
fn manifest_without_github_platform_selection_still_parses() {
    let contents = r#"
name = "rg"
version = "14.1.0"

[source.github]
repo = "BurntSushi/ripgrep"
channel = "stable"

[platform.macos-aarch64]
url = "https://example.invalid/rg-14.1.0-aarch64.tar.gz"
checksum = "sha256:2222222222222222222222222222222222222222222222222222222222222222"
archive = "tar.gz"
binaries = ["rg"]
"#;

    let manifest = Manifest::from_toml(contents).unwrap();
    let github = manifest.source.as_ref().unwrap().github.as_ref().unwrap();

    assert_eq!(github.repo, "BurntSushi/ripgrep");
    assert_eq!(github.channel, "stable");
    assert!(github.platform.is_empty());
}

#[test]
fn run_with_paths_routes_sync_to_validation_error() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let cli = Cli::try_parse_from(["hive", "sync", "ripgrep"]).unwrap();

    let error = app::run_with_paths(cli, paths).unwrap_err();
    assert!(error.contains("invalid GitHub repo"));
}

#[test]
fn run_capture_routes_sync_to_validation_error() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let cli = Cli::try_parse_from(["hive", "sync", "ripgrep"]).unwrap();

    let error = app::run_capture(cli, paths).unwrap_err();
    assert!(error.contains("invalid GitHub repo"));
}

#[test]
fn stable_channel_skips_drafts_and_prereleases() {
    let server = tests_support::spawn_github_server(vec![
        tests_support::release_json("v2.0.0-beta.1", true, false, vec![]),
        tests_support::release_json("v2.0.0", false, false, vec![]),
        tests_support::release_json("v1.9.0", false, false, vec![]),
    ]);

    let client = GitHubClient::new(server.api_base(), proxy::build_http_client().unwrap());
    let release = client
        .latest_release("BurntSushi/ripgrep", "stable")
        .unwrap();

    assert_eq!(release.tag_name, "v2.0.0");
}

#[test]
fn nightly_channel_allows_prereleases() {
    let server = tests_support::spawn_github_server(vec![
        tests_support::release_json("v2.0.0-beta.1", true, false, vec![]),
        tests_support::release_json("v1.9.0", false, false, vec![]),
    ]);

    let client = GitHubClient::new(server.api_base(), proxy::build_http_client().unwrap());
    let release = client
        .latest_release("BurntSushi/ripgrep", "nightly")
        .unwrap();

    assert_eq!(release.tag_name, "v2.0.0-beta.1");
}

#[test]
fn sync_rejects_invalid_hive_http_proxy_for_github_requests() {
    let _env = tests_support::lock_env();
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![],
    )]);

    unsafe {
        std::env::set_var("HIVE_HTTP_PROXY", "://bad-proxy");
    }
    let error =
        sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap_err();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
    }

    assert!(error.contains("HIVE_HTTP_PROXY"));
    assert!(error.contains("invalid proxy URL"));
}

#[test]
fn sync_rejects_invalid_hive_insecure_ssl_value_for_github_requests() {
    let _env = tests_support::lock_env();
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());

    unsafe {
        std::env::set_var("HIVE_INSECURE_SSL", "maybe");
    }
    let error = sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", "http://127.0.0.1:9")
        .unwrap_err();
    unsafe {
        std::env::remove_var("HIVE_INSECURE_SSL");
    }

    assert!(error.contains("HIVE_INSECURE_SSL"));
    assert!(error.contains("invalid boolean value"));
}

#[test]
fn first_sync_creates_manifest_with_stable_channel_and_checksum() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let archive_name = "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz";
    let archive_path = tests_support::write_named_tar_gz(temp.path(), archive_name, "rg");
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![tests_support::asset_json(
            archive_name,
            &tests_support::file_url(&archive_path),
        )],
    )]);

    sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap();

    let manifest = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    assert!(manifest.contains("version = \"14.1.0\""));
    assert!(manifest.contains("repo = \"BurntSushi/ripgrep\""));
    assert!(manifest.contains("channel = \"stable\""));
    assert!(manifest.contains("checksum = \"sha256:"));
}

#[test]
fn first_sync_uses_prompted_asset_for_current_platform() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current_archive_name = tests_support::platform_archive_name("nvim", "0.10.0");
    let current_archive_path =
        tests_support::write_named_tar_gz(temp.path(), &current_archive_name, "nvim");
    let other_archive_name = match tests_support::current_platform_key() {
        "linux-x86_64" => "nvim-0.10.0-aarch64-unknown-linux-musl.tar.gz",
        "linux-aarch64" => "nvim-0.10.0-x86_64-unknown-linux-musl.tar.gz",
        "macos-x86_64" => "nvim-0.10.0-aarch64-apple-darwin.tar.gz",
        "macos-aarch64" => "nvim-0.10.0-x86_64-apple-darwin.tar.gz",
        _ => panic!("unsupported test host"),
    };
    let other_archive_path =
        tests_support::write_named_tar_gz(temp.path(), other_archive_name, "nvim");
    let prompts = tests_support::ScriptedSyncPrompts::new(&["2", "bin/nvim"]);
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v0.10.0",
        false,
        false,
        vec![
            tests_support::asset_json(
                other_archive_name,
                &tests_support::file_url(&other_archive_path),
            ),
            tests_support::asset_json(
                &current_archive_name,
                &tests_support::file_url(&current_archive_path),
            ),
        ],
    )]);

    sync::sync_repo_with_api_base_and_prompt(&paths, "neovim/neovim", server.api_base(), &prompts)
        .unwrap();

    let manifest = fs::read_to_string(paths.manifest_dirs[0].join("neovim.toml")).unwrap();
    assert!(manifest.contains("version = \"0.10.0\""));
    assert!(manifest.contains("repo = \"neovim/neovim\""));
    assert!(manifest.contains("channel = \"stable\""));
    assert!(manifest.contains(&format!(
        "[source.github.platform.{}]",
        tests_support::current_platform_key()
    )));
    assert!(manifest.contains(&format!("asset = \"{current_archive_name}\"")));
    assert!(manifest.contains("binaries = [\"bin/nvim\"]"));
}

#[test]
fn sync_preserves_existing_binaries_and_rejects_repo_mismatch() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::write_manifest_with_github_source_and_binaries(
        &paths,
        "ripgrep",
        "14.0.0",
        "BurntSushi/ripgrep",
        "nightly",
        &["bin/rg"],
    );

    let error =
        sync::sync_repo_with_api_base(&paths, "someone/ripgrep", "http://127.0.0.1:9").unwrap_err();

    assert!(error.contains("stored GitHub repo does not match"));
    let manifest = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    assert!(manifest.contains("binaries = [\"bin/rg\"]"));
    assert!(manifest.contains("repo = \"BurntSushi/ripgrep\""));
}

#[test]
fn sync_leaves_existing_manifest_unchanged_when_asset_mapping_is_ambiguous() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::write_manifest_with_github_source_and_binaries(
        &paths,
        "ripgrep",
        "14.0.0",
        "BurntSushi/ripgrep",
        "stable",
        &["rg"],
    );
    let before = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![tests_support::asset_json(
            "ripgrep-14.1.0-universal.tar.gz",
            "https://example.invalid/ripgrep-14.1.0-universal.tar.gz",
        )],
    )]);

    let error =
        sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap_err();

    assert!(error.contains("could not map GitHub assets"));
    let after = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    assert_eq!(after, before);
}

#[test]
fn sync_leaves_existing_manifest_unchanged_when_release_drops_existing_platforms() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::write_manifest_with_github_source_platforms(
        &paths,
        "ripgrep",
        "14.0.0",
        "BurntSushi/ripgrep",
        "stable",
        &[
            (
                "linux-x86_64",
                "https://example.invalid/linux.tar.gz",
                "sha256:linux",
                &["rg"],
            ),
            (
                "macos-x86_64",
                "https://example.invalid/macos.tar.gz",
                "sha256:macos",
                &["rg"],
            ),
        ],
    );
    let before = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    let archive_name = "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz";
    let archive_path = tests_support::write_named_tar_gz(temp.path(), archive_name, "rg");
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![tests_support::asset_json(
            archive_name,
            &tests_support::file_url(&archive_path),
        )],
    )]);

    let error =
        sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap_err();

    assert!(error.contains("could not map GitHub assets"));
    let after = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    assert_eq!(after, before);
}

#[test]
fn sync_rejects_ambiguous_duplicate_platform_assets() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let linux_a = tests_support::write_named_tar_gz(
        temp.path(),
        "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz",
        "rg",
    );
    let linux_b = tests_support::write_named_tar_gz(
        temp.path(),
        "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz",
        "rg",
    );
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![
            tests_support::asset_json(
                "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz",
                &tests_support::file_url(&linux_a),
            ),
            tests_support::asset_json(
                "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz",
                &tests_support::file_url(&linux_b),
            ),
        ],
    )]);

    let error =
        sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap_err();

    assert!(error.contains("could not map GitHub assets"));
    assert!(!paths.manifest_dirs[0].join("ripgrep.toml").exists());
}

#[test]
fn sync_is_noop_when_release_and_artifacts_are_unchanged() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let archive_name = tests_support::platform_archive_name("ripgrep", "14.1.0");
    let archive_path = tests_support::write_named_tar_gz(temp.path(), &archive_name, "ripgrep");
    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&archive_path).unwrap())
    );
    tests_support::write_manifest_with_github_source_and_checksum(
        &paths,
        "ripgrep",
        "14.1.0",
        "BurntSushi/ripgrep",
        "stable",
        &tests_support::file_url(&archive_path),
        &checksum,
    );
    let before = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![tests_support::asset_json(
            &archive_name,
            &tests_support::file_url(&archive_path),
        )],
    )]);

    sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap();

    let after = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    assert_eq!(after, before);
}

#[test]
fn readme_sync_example_matches_cli_shape() {
    let readme = fs::read_to_string("README.md").unwrap();
    assert!(readme.contains("hive sync BurntSushi/ripgrep"));
    assert!(readme.contains("[source.github]"));
    assert!(readme.contains("channel = \"stable\""));
}
