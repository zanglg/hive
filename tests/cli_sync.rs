#[path = "support/mod.rs"]
mod tests_support;

use clap::Parser;
use hive::{
    app,
    cli::{Cli, Commands},
    github::GitHubClient,
    manifest::{Artifact, GitHubPlatformSelection, Manifest},
    proxy, sync,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
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
    let prompts = tests_support::ScriptedSyncPrompts::new(&["1", "rg"]);

    sync::sync_repo_with_api_base_and_prompt(
        &paths,
        "BurntSushi/ripgrep",
        server.api_base(),
        &prompts,
    )
    .unwrap();

    let manifest = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    assert!(manifest.contains("version = \"14.1.0\""));
    assert!(manifest.contains("repo = \"BurntSushi/ripgrep\""));
    assert!(manifest.contains("channel = \"stable\""));
    assert!(manifest.contains(&format!(
        "[source.github.platform.{}]",
        tests_support::current_platform_key()
    )));
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
fn cli_sync_routes_through_terminal_prompts_for_current_platform() {
    let _env = tests_support::lock_env();
    let temp = tempdir().unwrap();
    let home = temp.path();
    let manifest_path = home.join(".config/hive/manifests/neovim.toml");
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

    let mut child = Command::new(env!("CARGO_BIN_EXE_hive"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("HOME", home)
        .env("HIVE_GITHUB_API_BASE", server.api_base())
        .arg("sync")
        .arg("neovim/neovim")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"2\nbin/nvim\n")
        .unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = fs::read_to_string(manifest_path).unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Select asset"));
    assert!(manifest.contains(&format!("asset = \"{current_archive_name}\"")));
    assert!(manifest.contains("binaries = [\"bin/nvim\"]"));
}

#[test]
fn sync_preserves_other_platform_artifacts_when_updating_current_platform() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current_platform = tests_support::current_platform_key();
    let (other_platform, other_asset_name) = match current_platform {
        "linux-x86_64" => ("macos-aarch64", "nvim-macos-aarch64.zip"),
        "linux-aarch64" => ("macos-x86_64", "nvim-macos-x86_64.zip"),
        "macos-x86_64" => ("linux-aarch64", "nvim-linux-aarch64.tar.gz"),
        "macos-aarch64" => ("linux-x86_64", "nvim-linux-x86_64.tar.gz"),
        _ => panic!("unsupported test host"),
    };
    let current_archive_name = "nvim-portable.tar.gz";
    let current_archive_path =
        tests_support::write_named_tar_gz(temp.path(), current_archive_name, "nvim");
    let current_archive_url = tests_support::file_url(&current_archive_path);
    let current_checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&current_archive_path).unwrap())
    );
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("neovim.toml"),
        format!(
            r#"
name = "neovim"
version = "0.9.0"

[source.github]
repo = "neovim/neovim"
channel = "stable"

[source.github.platform.{current_platform}]
asset = "nvim-old-current.tar.gz"
binaries = ["old/nvim"]

[source.github.platform.{other_platform}]
asset = "{other_asset_name}"
binaries = ["bin/nvim-other"]

[platform.{current_platform}]
url = "https://example.invalid/old-current.tar.gz"
checksum = "sha256:old-current"
archive = "tar.gz"
binaries = ["old/nvim"]

[platform.{other_platform}]
url = "https://example.invalid/other-platform.zip"
checksum = "sha256:other-platform"
archive = "zip"
binaries = ["bin/nvim-other"]
"#
        ),
    )
    .unwrap();
    let prompts =
        tests_support::ScriptedSyncPrompts::new(&[current_archive_name, "bin/nvim,bin/nvimdiff"]);
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v0.10.0",
        false,
        false,
        vec![tests_support::asset_json(
            current_archive_name,
            &current_archive_url,
        )],
    )]);

    sync::sync_repo_with_api_base_and_prompt(&paths, "neovim/neovim", server.api_base(), &prompts)
        .unwrap();

    let manifest = Manifest::from_toml(
        &fs::read_to_string(paths.manifest_dirs[0].join("neovim.toml")).unwrap(),
    )
    .unwrap();
    let github = manifest.source.as_ref().unwrap().github.as_ref().unwrap();
    let current_selection = github.platform.get(current_platform).unwrap();
    let other_selection = github.platform.get(other_platform).unwrap();
    let current_artifact = manifest.platform.get(current_platform).unwrap();
    let other_artifact = manifest.platform.get(other_platform).unwrap();

    assert_eq!(manifest.version, "0.10.0");
    assert_eq!(current_selection.asset, current_archive_name);
    assert_eq!(
        current_selection.binaries,
        vec!["bin/nvim".to_string(), "bin/nvimdiff".to_string()]
    );
    assert_eq!(other_selection.asset, other_asset_name);
    assert_eq!(other_selection.binaries, vec!["bin/nvim-other".to_string()]);
    assert_eq!(current_artifact.url, current_archive_url);
    assert_eq!(current_artifact.checksum, current_checksum);
    assert_eq!(current_artifact.archive, "tar.gz");
    assert_eq!(
        current_artifact.binaries,
        vec!["bin/nvim".to_string(), "bin/nvimdiff".to_string()]
    );
    assert_eq!(
        other_artifact.url,
        "https://example.invalid/other-platform.zip"
    );
    assert_eq!(other_artifact.checksum, "sha256:other-platform");
    assert_eq!(other_artifact.archive, "zip");
    assert_eq!(other_artifact.binaries, vec!["bin/nvim-other".to_string()]);
}

#[test]
fn sync_uses_existing_current_platform_artifact_url_when_selection_is_missing() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current_platform = tests_support::current_platform_key();
    let archive_name = tests_support::platform_archive_name("ripgrep", "14.1.0");
    let archive_path = tests_support::write_named_tar_gz(temp.path(), &archive_name, "rg");
    let archive_url = tests_support::file_url(&archive_path);
    let mut manifest = tests_support::manifest_with_github_source(
        "ripgrep",
        "14.0.0",
        "BurntSushi/ripgrep",
        "stable",
    );
    let artifact = manifest.platform.get_mut(current_platform).unwrap();
    artifact.url = archive_url.clone();
    artifact.binaries = vec!["bin/rg".to_string()];
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("ripgrep.toml"),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![tests_support::asset_json(&archive_name, &archive_url)],
    )]);

    sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap();

    let manifest = Manifest::from_toml(
        &fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap(),
    )
    .unwrap();
    let github = manifest.source.as_ref().unwrap().github.as_ref().unwrap();
    let selection = github.platform.get(current_platform).unwrap();
    let artifact = manifest.platform.get(current_platform).unwrap();

    assert_eq!(manifest.version, "14.1.0");
    assert_eq!(selection.asset, archive_name);
    assert_eq!(selection.binaries, vec!["bin/rg".to_string()]);
    assert_eq!(artifact.url, archive_url);
    assert_eq!(artifact.binaries, vec!["bin/rg".to_string()]);
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
fn sync_rejects_prompted_asset_with_unsupported_archive_format() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let prompts = tests_support::ScriptedSyncPrompts::new(&["1"]);
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![tests_support::asset_json(
            "ripgrep-14.1.0-checksums.txt",
            "https://example.invalid/ripgrep-14.1.0-checksums.txt",
        )],
    )]);

    let error = sync::sync_repo_with_api_base_and_prompt(
        &paths,
        "BurntSushi/ripgrep",
        server.api_base(),
        &prompts,
    )
    .unwrap_err();

    assert!(error.contains("unsupported archive format"));
    assert!(!paths.manifest_dirs[0].join("ripgrep.toml").exists());
}

#[test]
fn sync_leaves_existing_manifest_unchanged_when_saved_asset_is_missing_from_release() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current_platform = tests_support::current_platform_key().to_string();
    let selected_archive_name = tests_support::platform_archive_name("ripgrep", "14.1.0");
    let mut manifest = tests_support::manifest_with_github_source(
        "ripgrep",
        "14.0.0",
        "BurntSushi/ripgrep",
        "stable",
    );
    manifest
        .source
        .as_mut()
        .unwrap()
        .github
        .as_mut()
        .unwrap()
        .platform
        .insert(
            current_platform.clone(),
            GitHubPlatformSelection {
                asset: selected_archive_name.clone(),
                binaries: vec!["rg".to_string()],
            },
        );
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("ripgrep.toml"),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
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

    assert!(error.contains(&format!(
        "selected asset `{selected_archive_name}` was not found in release"
    )));
    let after = fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap();
    assert_eq!(after, before);
}

#[test]
fn sync_preserves_other_platform_artifacts_with_saved_current_selection() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current_platform = tests_support::current_platform_key().to_string();
    let (other_platform, other_asset_name) = match current_platform.as_str() {
        "linux-x86_64" => ("macos-aarch64", "ripgrep-macos-aarch64.zip"),
        "linux-aarch64" => ("macos-x86_64", "ripgrep-macos-x86_64.zip"),
        "macos-x86_64" => ("linux-aarch64", "ripgrep-linux-aarch64.tar.gz"),
        "macos-aarch64" => ("linux-x86_64", "ripgrep-linux-x86_64.tar.gz"),
        _ => panic!("unsupported test host"),
    };
    let archive_name = tests_support::platform_archive_name("ripgrep", "14.1.0");
    let archive_path = tests_support::write_named_tar_gz(temp.path(), &archive_name, "rg");
    let archive_url = tests_support::file_url(&archive_path);
    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&archive_path).unwrap())
    );
    let mut manifest = tests_support::manifest_with_github_source(
        "ripgrep",
        "14.0.0",
        "BurntSushi/ripgrep",
        "stable",
    );
    manifest
        .source
        .as_mut()
        .unwrap()
        .github
        .as_mut()
        .unwrap()
        .platform
        .insert(
            current_platform.clone(),
            GitHubPlatformSelection {
                asset: archive_name.clone(),
                binaries: vec!["rg".to_string()],
            },
        );
    manifest
        .source
        .as_mut()
        .unwrap()
        .github
        .as_mut()
        .unwrap()
        .platform
        .insert(
            other_platform.to_string(),
            GitHubPlatformSelection {
                asset: other_asset_name.to_string(),
                binaries: vec!["rg-other".to_string()],
            },
        );
    manifest.platform.insert(
        other_platform.to_string(),
        Artifact {
            url: "https://example.invalid/other-platform.zip".to_string(),
            checksum: "sha256:other-platform".to_string(),
            archive: "zip".to_string(),
            binaries: vec!["rg-other".to_string()],
        },
    );
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("ripgrep.toml"),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![tests_support::asset_json(&archive_name, &archive_url)],
    )]);

    sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap();

    let manifest = Manifest::from_toml(
        &fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap(),
    )
    .unwrap();
    let github = manifest.source.as_ref().unwrap().github.as_ref().unwrap();
    let other_selection = github.platform.get(other_platform).unwrap();
    let current_artifact = manifest.platform.get(&current_platform).unwrap();
    let other_artifact = manifest.platform.get(other_platform).unwrap();

    assert_eq!(manifest.version, "14.1.0");
    assert_eq!(current_artifact.url, archive_url);
    assert_eq!(current_artifact.checksum, checksum);
    assert_eq!(other_selection.asset, other_asset_name);
    assert_eq!(other_selection.binaries, vec!["rg-other".to_string()]);
    assert_eq!(
        other_artifact.url,
        "https://example.invalid/other-platform.zip"
    );
    assert_eq!(other_artifact.checksum, "sha256:other-platform");
    assert_eq!(other_artifact.archive, "zip");
    assert_eq!(other_artifact.binaries, vec!["rg-other".to_string()]);
}

#[test]
fn sync_uses_exact_saved_filename_when_release_contains_similar_assets() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current_platform = tests_support::current_platform_key().to_string();
    let selected_archive_name = tests_support::platform_archive_name("ripgrep", "14.1.0");
    let selected_archive_path =
        tests_support::write_named_tar_gz(temp.path(), &selected_archive_name, "rg");
    let selected_archive_url = tests_support::file_url(&selected_archive_path);
    let selected_checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&selected_archive_path).unwrap())
    );
    let similar_archive_name = selected_archive_name.replacen(".tar.gz", "-symbols.tar.gz", 1);
    let similar_archive_path =
        tests_support::write_named_tar_gz(temp.path(), &similar_archive_name, "rg");
    let mut manifest = tests_support::manifest_with_github_source(
        "ripgrep",
        "14.0.0",
        "BurntSushi/ripgrep",
        "stable",
    );
    manifest
        .source
        .as_mut()
        .unwrap()
        .github
        .as_mut()
        .unwrap()
        .platform
        .insert(
            current_platform.clone(),
            GitHubPlatformSelection {
                asset: selected_archive_name.clone(),
                binaries: vec!["rg".to_string()],
            },
        );
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("ripgrep.toml"),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
    let server = tests_support::spawn_github_server(vec![tests_support::release_json(
        "v14.1.0",
        false,
        false,
        vec![
            tests_support::asset_json(&selected_archive_name, &selected_archive_url),
            tests_support::asset_json(
                &similar_archive_name,
                &tests_support::file_url(&similar_archive_path),
            ),
        ],
    )]);

    sync::sync_repo_with_api_base(&paths, "BurntSushi/ripgrep", server.api_base()).unwrap();

    let manifest = Manifest::from_toml(
        &fs::read_to_string(paths.manifest_dirs[0].join("ripgrep.toml")).unwrap(),
    )
    .unwrap();
    let artifact = manifest.platform.get(&current_platform).unwrap();

    assert_eq!(artifact.url, selected_archive_url);
    assert_eq!(artifact.checksum, selected_checksum);
}

#[test]
fn sync_is_noop_when_release_and_artifacts_are_unchanged() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current_platform = tests_support::current_platform_key().to_string();
    let archive_name = tests_support::platform_archive_name("ripgrep", "14.1.0");
    let archive_path = tests_support::write_named_tar_gz(temp.path(), &archive_name, "ripgrep");
    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&archive_path).unwrap())
    );
    let mut manifest = tests_support::manifest_with_github_source(
        "ripgrep",
        "14.1.0",
        "BurntSushi/ripgrep",
        "stable",
    );
    manifest
        .source
        .as_mut()
        .unwrap()
        .github
        .as_mut()
        .unwrap()
        .platform
        .insert(
            current_platform.clone(),
            GitHubPlatformSelection {
                asset: archive_name.clone(),
                binaries: vec!["ripgrep".to_string()],
            },
        );
    let artifact = manifest.platform.get_mut(&current_platform).unwrap();
    artifact.url = tests_support::file_url(&archive_path);
    artifact.checksum = checksum.clone();
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("ripgrep.toml"),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
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
