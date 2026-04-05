#[path = "support/mod.rs"]
mod tests_support;

use clap::Parser;
use hive::{
    app::{self, InstallPrompts},
    cli::{Cli, Commands},
    config::HivePaths,
    installer::{ArchiveKind, Installer},
    manifest::{Manifest, ManifestRepository},
    platform::Platform,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn parses_install_command_with_package_name() {
    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();

    match cli.command {
        Commands::Install { package } => assert_eq!(package, "rg"),
        _ => panic!("expected install command"),
    }
}

#[test]
fn builds_default_hive_paths_from_home_directory() {
    let paths = HivePaths::from_home(PathBuf::from("/tmp/alice"));

    assert_eq!(
        paths.manifest_dirs,
        vec![PathBuf::from("/tmp/alice/.config/hive/manifests")]
    );
    assert_eq!(
        paths.package_store,
        PathBuf::from("/tmp/alice/.local/share/hive/pkgs")
    );
    assert_eq!(
        paths.state_dir,
        PathBuf::from("/tmp/alice/.local/share/hive/state")
    );
    assert_eq!(paths.shim_dir, PathBuf::from("/tmp/alice/.local/bin/hive"));
}

#[test]
fn parses_supported_platform_keys() {
    assert_eq!(
        "linux-x86_64".parse::<Platform>().unwrap(),
        Platform::LinuxX86_64
    );
    assert_eq!(
        "linux-aarch64".parse::<Platform>().unwrap(),
        Platform::LinuxAarch64
    );
    assert_eq!(
        "macos-x86_64".parse::<Platform>().unwrap(),
        Platform::MacosX86_64
    );
    assert_eq!(
        "macos-aarch64".parse::<Platform>().unwrap(),
        Platform::MacosAarch64
    );
}

#[test]
fn parses_manifest_and_resolves_platform_artifact() {
    let contents = include_str!("fixtures/manifests/rg.toml");
    let manifest = Manifest::from_toml(contents).unwrap();
    let artifact = manifest.artifact_for(Platform::LinuxX86_64).unwrap();

    assert_eq!(manifest.name, "rg");
    assert_eq!(manifest.version, "14.1.0");
    assert_eq!(artifact.archive, "tar.gz");
    assert_eq!(artifact.binaries, vec!["rg"]);
}

#[test]
fn parses_manifest_with_optional_github_source() {
    let contents = r#"
name = "rg"
version = "14.1.0"

[source.github]
repo = "BurntSushi/ripgrep"
channel = "stable"

[platform.linux-x86_64]
url = "https://example.invalid/rg.tar.gz"
checksum = "sha256:abc"
archive = "tar.gz"
binaries = ["rg"]
"#;

    let manifest = Manifest::from_toml(contents).unwrap();
    let source = manifest.source.as_ref().unwrap().github.as_ref().unwrap();

    assert_eq!(source.repo, "BurntSushi/ripgrep");
    assert_eq!(source.channel, "stable");
}

#[test]
fn manifest_can_replace_current_platform_binaries_without_touching_other_platforms() {
    let current = tests_support::current_platform_key();
    let other = tests_support::alternate_platform_key();
    let contents = format!(
        r#"
name = "rg"
version = "14.1.0"

[platform.{current}]
url = "https://example.invalid/current.tar.gz"
checksum = "sha256:abc"
archive = "tar.gz"
binaries = ["old-bin"]

[platform.{other}]
url = "https://example.invalid/other.tar.gz"
checksum = "sha256:def"
archive = "tar.gz"
binaries = ["other-bin"]
"#
    );

    let mut manifest = Manifest::from_toml(&contents).unwrap();
    manifest
        .set_binaries_for_platform(
            current,
            vec!["bin/rg".to_string(), "bin/rga".to_string()],
        )
        .unwrap();

    let manifest = Manifest::from_toml(&manifest.to_toml().unwrap()).unwrap();

    assert_eq!(
        manifest.platform.get(current).unwrap().binaries,
        vec!["bin/rg", "bin/rga"]
    );
    assert_eq!(
        manifest.platform.get(other).unwrap().binaries,
        vec!["other-bin"]
    );
}

#[test]
fn renders_manifest_with_github_source_block_before_platforms() {
    let manifest =
        tests_support::manifest_with_github_source("rg", "14.1.0", "BurntSushi/ripgrep", "stable");
    let rendered = manifest.to_toml().unwrap();

    let source_block = "[source.github]\nrepo = \"BurntSushi/ripgrep\"\nchannel = \"stable\"";
    let platform_block = format!("[platform.{}]", tests_support::current_platform_key());

    let source_index = rendered.find(source_block).unwrap();
    let platform_index = rendered.find(&platform_block).unwrap();

    assert!(source_index < platform_index);
}

#[cfg(unix)]
#[test]
fn list_executable_candidates_returns_relative_paths_in_stable_order() {
    let temp = tempdir().unwrap();
    let install_root = temp.path().join("install");
    let alpha = install_root.join("bin/alpha");
    let beta = install_root.join("tools/beta");
    write_file_with_mode(&alpha, b"alpha", 0o755);
    write_file_with_mode(&beta, b"beta", 0o755);

    let candidates = hive::installer::list_executable_candidates(&install_root).unwrap();

    assert_eq!(candidates, vec!["bin/alpha", "tools/beta"]);
}

#[cfg(unix)]
#[test]
fn list_executable_candidates_ignores_non_executable_files() {
    let temp = tempdir().unwrap();
    let install_root = temp.path().join("install");
    let readme = install_root.join("README");
    write_file_with_mode(&readme, b"docs", 0o644);

    let candidates = hive::installer::list_executable_candidates(&install_root).unwrap();

    assert!(candidates.is_empty());
}

fn write_file_with_mode(path: &Path, contents: &[u8], mode: u32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();

    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions).unwrap();
    }
}

#[test]
fn finds_manifest_from_flat_or_nested_layout() {
    let repo = ManifestRepository::new(vec![PathBuf::from("tests/fixtures/manifests")]);
    let manifest_path = repo.find("rg").unwrap();

    assert!(manifest_path.ends_with("tests/fixtures/manifests/rg.toml"));
}

#[test]
fn fails_when_package_name_is_ambiguous() {
    let repo = ManifestRepository::new(vec![
        PathBuf::from("tests/fixtures/manifests"),
        PathBuf::from("tests/fixtures/manifests/ambiguous"),
    ]);

    let error = repo.find("rg").unwrap_err();
    assert!(error.contains("ambiguous"));
}

#[test]
fn installs_archive_into_versioned_package_store() {
    let temp = tempdir().unwrap();
    let archive_path = temp.path().join("rg.tar.gz");
    let source_dir = temp.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("rg"), "stub-binary").unwrap();
    tests_support::write_tar_gz(&archive_path, &source_dir, "rg");

    let bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(bytes));
    let declared_binaries = vec!["rg".to_string()];

    let installer = Installer::new(temp.path().join("pkgs"));
    let install_dir = installer
        .install_archive(
            "rg",
            "14.1.0",
            &archive_path,
            &checksum,
            ArchiveKind::TarGz,
            &declared_binaries,
        )
        .unwrap();

    assert!(install_dir.join("rg").exists());
}

#[test]
fn install_strips_single_wrapper_directory_before_validating_binaries() {
    let temp = tempdir().unwrap();
    let archive_path = temp.path().join("rg.tar.gz");
    let source_dir = temp.path().join("source");
    fs::create_dir_all(source_dir.join("bin")).unwrap();
    fs::write(source_dir.join("bin/gh"), "stub-binary").unwrap();
    tests_support::write_tar_gz_with_wrapper(&archive_path, &source_dir, "release");

    let bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(bytes));
    let declared_binaries = vec!["bin/gh".to_string()];

    let installer = Installer::new(temp.path().join("pkgs"));
    let install_dir = installer
        .install_archive(
            "gh",
            "2.0.0",
            &archive_path,
            &checksum,
            ArchiveKind::TarGz,
            &declared_binaries,
        )
        .unwrap();

    assert!(install_dir.join("bin/gh").exists());
    assert!(!install_dir.join("release").exists());
}

#[test]
fn install_keeps_single_wrapper_directory_unchanged_when_declared_binaries_only_exist_via_symlink()
{
    let temp = tempdir().unwrap();
    let archive_path = temp.path().join("rg.tar.gz");
    let payload_dir = temp.path().join("payload");
    fs::create_dir_all(&payload_dir).unwrap();
    fs::write(payload_dir.join("sh"), "stub-binary").unwrap();
    tests_support::write_tar_gz_with_symlink(&archive_path, "release/bin", &payload_dir);

    let bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(bytes));
    let declared_binaries = vec!["bin/sh".to_string()];

    let installer = Installer::new(temp.path().join("pkgs"));
    let install_dir = installer
        .install_archive(
            "rg",
            "14.1.0",
            &archive_path,
            &checksum,
            ArchiveKind::TarGz,
            &declared_binaries,
        )
        .unwrap();

    assert!(
        install_dir
            .join("release/bin")
            .symlink_metadata()
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    );
    assert!(!install_dir.join("bin/sh").exists());
}

#[test]
fn install_keeps_single_wrapper_directory_unchanged_when_declared_binaries_are_missing() {
    let temp = tempdir().unwrap();
    let archive_path = temp.path().join("gh.tar.gz");
    let source_dir = temp.path().join("source");
    fs::create_dir_all(source_dir.join("share")).unwrap();
    fs::write(source_dir.join("share/notes.txt"), "docs").unwrap();
    tests_support::write_tar_gz_with_wrapper(&archive_path, &source_dir, "release");

    let bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(bytes));
    let declared_binaries = vec!["bin/gh".to_string()];

    let installer = Installer::new(temp.path().join("pkgs"));
    let install_dir = installer
        .install_archive(
            "gh",
            "2.0.0",
            &archive_path,
            &checksum,
            ArchiveKind::TarGz,
            &declared_binaries,
        )
        .unwrap();

    assert!(install_dir.join("release/share/notes.txt").exists());
    assert!(!install_dir.join("share/notes.txt").exists());
}

#[test]
fn install_rejects_invalid_hive_http_proxy_for_http_downloads() {
    let _env = tests_support::lock_env();
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let archive_name = "rg.tar.gz";
    let archive_path = tests_support::write_named_tar_gz(temp.path(), archive_name, "rg");
    let archive_bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(&archive_bytes));
    let server = tests_support::spawn_http_server(archive_bytes, "application/gzip");

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
        .replace(&format!("file://{}", archive_path.display()), server.url());
    fs::write(&manifest_path, manifest).unwrap();

    unsafe {
        std::env::set_var("HIVE_HTTP_PROXY", "://bad-proxy");
    }
    let error = app::run_capture(
        Cli::try_parse_from(["hive", "install", "rg"]).unwrap(),
        paths,
    )
    .unwrap_err();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
    }

    assert!(error.contains("HIVE_HTTP_PROXY"));
    assert!(error.contains("invalid proxy URL"));
}

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

#[test]
fn install_command_rejects_symlinked_declared_binaries() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_symlink_binary_fixture(&paths, "rg", "14.1.0");

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    let error = app::run_with_paths(cli, paths.clone()).unwrap_err();

    assert!(error.contains("declared binary missing"));
    assert!(!paths.package_store.join("rg/14.1.0").exists());
}

#[test]
fn install_does_not_treat_symlink_to_directory_as_wrapper_directory() {
    let temp = tempdir().unwrap();
    let archive_path = temp.path().join("rg.tar.gz");
    let payload_dir = temp.path().join("payload");
    fs::create_dir_all(&payload_dir).unwrap();
    fs::write(payload_dir.join("rg"), "stub-binary").unwrap();
    tests_support::write_tar_gz_with_symlink(&archive_path, "release", &payload_dir);

    let bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(bytes));
    let declared_binaries = vec!["rg".to_string()];

    let installer = Installer::new(temp.path().join("pkgs"));
    let install_dir = installer
        .install_archive(
            "rg",
            "14.1.0",
            &archive_path,
            &checksum,
            ArchiveKind::TarGz,
            &declared_binaries,
        )
        .unwrap();

    assert!(payload_dir.join("rg").exists());
    assert!(
        install_dir
            .join("release")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!install_dir.join("rg").exists());
}

#[test]
fn install_keeps_flat_archive_layout_unchanged() {
    let temp = tempdir().unwrap();
    let archive_path = temp.path().join("rg.tar.gz");
    let source_dir = temp.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("rg"), "stub-binary").unwrap();
    fs::write(source_dir.join("README"), "docs").unwrap();
    let tar_gz = fs::File::create(&archive_path).unwrap();
    let encoder = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder
        .append_path_with_name(source_dir.join("rg"), "rg")
        .unwrap();
    builder
        .append_path_with_name(source_dir.join("README"), "README")
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();

    let bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(bytes));
    let declared_binaries = vec!["rg".to_string()];

    let installer = Installer::new(temp.path().join("pkgs"));
    let install_dir = installer
        .install_archive(
            "rg",
            "14.1.0",
            &archive_path,
            &checksum,
            ArchiveKind::TarGz,
            &declared_binaries,
        )
        .unwrap();

    assert!(install_dir.join("rg").exists());
    assert!(install_dir.join("README").exists());
}

#[test]
fn installs_tar_xz_archive_into_versioned_package_store() {
    let temp = tempdir().unwrap();
    let archive_path = temp.path().join("hx.tar.xz");
    let source_dir = temp.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("hx"), "stub-binary").unwrap();
    tests_support::write_tar_xz(&archive_path, &source_dir, "hx");

    let bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(bytes));
    let declared_binaries = vec!["hx".to_string()];

    let installer = Installer::new(temp.path().join("pkgs"));
    let install_dir = installer
        .install_archive(
            "hx",
            "25.07.1",
            &archive_path,
            &checksum,
            ArchiveKind::parse("tar.xz").unwrap(),
            &declared_binaries,
        )
        .unwrap();

    assert!(install_dir.join("hx").exists());
}

#[test]
fn install_command_supports_tar_xz_archives_via_manifest() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture_tar_xz(&paths, "hx", "25.07.1");

    let cli = Cli::try_parse_from(["hive", "install", "hx"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert!(paths.package_store.join("hx/25.07.1/hx").exists());
    assert_eq!(
        fs::read_link(paths.package_store.join("hx/current")).unwrap(),
        paths.package_store.join("hx/25.07.1")
    );
    assert_eq!(
        fs::read_link(paths.shim_dir.join("hx")).unwrap(),
        paths.package_store.join("hx/current/hx")
    );
}

#[test]
fn install_command_resolves_manifest_and_activates_first_version() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture(&paths, "rg", "14.1.0");

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert!(paths.package_store.join("rg/14.1.0/rg").exists());
    assert_eq!(
        fs::read_link(paths.package_store.join("rg/current")).unwrap(),
        paths.package_store.join("rg/14.1.0")
    );
    assert_eq!(
        fs::read_link(paths.shim_dir.join("rg")).unwrap(),
        paths.package_store.join("rg/current/rg")
    );
}

#[test]
#[cfg(unix)]
fn install_prompts_for_missing_binaries_and_persists_selection() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture_without_binaries(&paths, "rg", "14.1.0", &["bin/rg"]);

    let prompts = tests_support::ScriptedInstallPrompts::new(&["1"]);
    app::install_package_with_prompts(&paths, "rg", &prompts).unwrap();

    let manifest = fs::read_to_string(paths.manifest_dirs[0].join("rg.toml")).unwrap();
    assert!(manifest.contains("binaries = [\"bin/rg\"]"));
    assert!(paths.package_store.join("rg/14.1.0/bin/rg").exists());
}

#[test]
#[cfg(unix)]
fn install_fails_when_missing_binaries_archive_has_no_executable_candidates() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let archive_path = temp.path().join("rg.tar.gz");
    let source_dir = temp.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("README"), "docs").unwrap();
    tests_support::write_tar_gz(&archive_path, &source_dir, "README");
    let checksum = format!("sha256:{:x}", Sha256::digest(fs::read(&archive_path).unwrap()));
    let current = tests_support::current_platform_key();
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("rg.toml"),
        format!(
            "name = \"rg\"\nversion = \"14.1.0\"\n\n[platform.{current}]\nurl = \"file://{}\"\nchecksum = \"{checksum}\"\narchive = \"tar.gz\"\nbinaries = []\n",
            archive_path.display()
        ),
    )
    .unwrap();

    let prompts = tests_support::ScriptedInstallPrompts::new(&["1"]);
    let error = app::install_package_with_prompts(&paths, "rg", &prompts).unwrap_err();

    assert!(error.contains("no executable candidates found"));
    assert!(fs::read_to_string(paths.manifest_dirs[0].join("rg.toml"))
        .unwrap()
        .contains("binaries = []"));
    assert!(!paths.package_store.join("rg/14.1.0").exists());
}

#[test]
#[cfg(unix)]
fn install_fails_when_missing_binaries_selection_is_empty() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture_without_binaries(&paths, "rg", "14.1.0", &["bin/rg"]);

    struct EmptySelectionPrompts;

    impl InstallPrompts for EmptySelectionPrompts {
        fn select_binaries(
            &self,
            _package: &str,
            _candidates: &[String],
        ) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
    }

    let prompts = EmptySelectionPrompts;
    let error = app::install_package_with_prompts(&paths, "rg", &prompts).unwrap_err();

    assert!(error.contains("binary selection cannot be empty"));
    assert!(fs::read_to_string(paths.manifest_dirs[0].join("rg.toml"))
        .unwrap()
        .contains("binaries = []"));
}

#[test]
#[cfg(unix)]
fn install_preserves_other_platform_artifacts_when_persisting_selected_binaries() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current = tests_support::current_platform_key();
    let other = tests_support::alternate_platform_key();
    let archive_path = temp.path().join("rg.tar.gz");
    let source_dir = temp.path().join("source");
    write_file_with_mode(&source_dir.join("bin/rg"), b"rg", 0o755);
    tests_support::write_tar_gz_files(&archive_path, &[(source_dir.join("bin/rg").as_path(), "bin/rg")]);
    let checksum = format!("sha256:{:x}", Sha256::digest(fs::read(&archive_path).unwrap()));
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("rg.toml"),
        format!(
            "name = \"rg\"\nversion = \"14.1.0\"\n\n[platform.{current}]\nurl = \"file://{}\"\nchecksum = \"{checksum}\"\narchive = \"tar.gz\"\nbinaries = []\n\n[platform.{other}]\nurl = \"https://example.invalid/other.tar.gz\"\nchecksum = \"sha256:other\"\narchive = \"tar.gz\"\nbinaries = [\"other-bin\"]\n",
            archive_path.display()
        ),
    )
    .unwrap();

    let prompts = tests_support::ScriptedInstallPrompts::new(&["1"]);
    app::install_package_with_prompts(&paths, "rg", &prompts).unwrap();

    let manifest =
        Manifest::from_toml(&fs::read_to_string(paths.manifest_dirs[0].join("rg.toml")).unwrap())
            .unwrap();
    assert_eq!(
        manifest.platform.get(current).unwrap().binaries,
        vec!["bin/rg"]
    );
    assert_eq!(
        manifest.platform.get(other).unwrap().binaries,
        vec!["other-bin"]
    );
}

#[test]
#[cfg(unix)]
fn install_keeps_follow_up_runs_noninteractive_after_persisting_binaries() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture_without_binaries(&paths, "rg", "14.1.0", &["bin/rg"]);

    let prompts = tests_support::ScriptedInstallPrompts::new(&["1"]);
    app::install_package_with_prompts(&paths, "rg", &prompts).unwrap();

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert_eq!(
        fs::read_link(paths.shim_dir.join("rg")).unwrap(),
        paths.package_store.join("rg/current/bin/rg")
    );
    let manifest = fs::read_to_string(paths.manifest_dirs[0].join("rg.toml")).unwrap();
    assert!(manifest.contains("binaries = [\"bin/rg\"]"));
}

#[test]
#[cfg(unix)]
fn install_updates_existing_github_platform_binaries_when_persisting_selection() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let current = tests_support::current_platform_key();
    let other = tests_support::alternate_platform_key();
    let archive_path = temp.path().join("rg.tar.gz");
    let source_dir = temp.path().join("source");
    write_file_with_mode(&source_dir.join("bin/rg"), b"rg", 0o755);
    tests_support::write_tar_gz_files(&archive_path, &[(source_dir.join("bin/rg").as_path(), "bin/rg")]);
    let checksum = format!("sha256:{:x}", Sha256::digest(fs::read(&archive_path).unwrap()));
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join("rg.toml"),
        format!(
            "name = \"rg\"\nversion = \"14.1.0\"\n\n[source.github]\nrepo = \"BurntSushi/ripgrep\"\nchannel = \"stable\"\n\n[source.github.platform.{current}]\nasset = \"{}\"\nbinaries = [\"old-bin\"]\n\n[source.github.platform.{other}]\nasset = \"other-asset\"\nbinaries = [\"other-github-bin\"]\n\n[platform.{current}]\nurl = \"file://{}\"\nchecksum = \"{checksum}\"\narchive = \"tar.gz\"\nbinaries = []\n\n[platform.{other}]\nurl = \"https://example.invalid/other.tar.gz\"\nchecksum = \"sha256:other\"\narchive = \"tar.gz\"\nbinaries = [\"other-bin\"]\n",
            tests_support::platform_archive_name("rg", "14.1.0"),
            archive_path.display()
        ),
    )
    .unwrap();

    let prompts = tests_support::ScriptedInstallPrompts::new(&["1"]);
    app::install_package_with_prompts(&paths, "rg", &prompts).unwrap();

    let manifest =
        Manifest::from_toml(&fs::read_to_string(paths.manifest_dirs[0].join("rg.toml")).unwrap())
            .unwrap();
    let github = manifest.source.unwrap().github.unwrap();
    assert_eq!(
        github.platform.get(current).unwrap().binaries,
        vec!["bin/rg"]
    );
    assert_eq!(
        github.platform.get(other).unwrap().binaries,
        vec!["other-github-bin"]
    );
}

#[test]
#[cfg(unix)]
fn install_noninteractive_missing_binaries_does_not_clobber_existing_same_version_dir() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture_without_binaries(&paths, "rg", "14.1.0", &["bin/rg"]);
    let existing_binary = paths.package_store.join("rg/14.1.0/bin/rg");
    write_file_with_mode(&existing_binary, b"existing-binary", 0o755);

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    let error = app::run_capture(cli, paths.clone()).unwrap_err();

    assert!(error.contains("manifest is missing binaries for the current platform"));
    assert_eq!(fs::read(&existing_binary).unwrap(), b"existing-binary");
}

#[test]
#[cfg(unix)]
fn install_restores_manifest_when_failure_happens_after_persistence() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture_without_binaries(&paths, "rg", "14.1.0", &["bin/rg"]);
    let manifest_path = paths.manifest_dirs[0].join("rg.toml");
    let original_manifest = fs::read_to_string(&manifest_path).unwrap();
    fs::create_dir_all(paths.shim_dir.parent().unwrap()).unwrap();
    fs::write(&paths.shim_dir, "blocker").unwrap();

    let prompts = tests_support::ScriptedInstallPrompts::new(&["1"]);
    let error = app::install_package_with_prompts(&paths, "rg", &prompts).unwrap_err();

    assert!(error.contains("failed to create"));
    assert_eq!(fs::read_to_string(&manifest_path).unwrap(), original_manifest);
    assert!(paths.package_store.join("rg/14.1.0").symlink_metadata().is_err());
}

#[test]
#[cfg(unix)]
fn install_blocks_when_stale_install_backup_already_exists() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture_without_binaries(&paths, "rg", "14.1.0", &["bin/rg"]);
    let existing_binary = paths.package_store.join("rg/14.1.0/bin/rg");
    write_file_with_mode(&existing_binary, b"existing-binary", 0o755);
    let stale_backup_binary = paths.package_store.join("rg/14.1.0.install-backup/bin/rg");
    write_file_with_mode(&stale_backup_binary, b"stale-backup", 0o755);

    let prompts = tests_support::ScriptedInstallPrompts::new(&["1"]);
    let error = app::install_package_with_prompts(&paths, "rg", &prompts).unwrap_err();

    assert!(error.contains("pre-existing install backup"));
    assert_eq!(fs::read(&existing_binary).unwrap(), b"existing-binary");
    assert_eq!(fs::read(&stale_backup_binary).unwrap(), b"stale-backup");
}

#[test]
#[cfg(unix)]
fn install_restores_existing_same_version_install_after_late_fallback_failure() {
    let _env = tests_support::lock_env();
    unsafe {
        std::env::remove_var("HIVE_HTTP_PROXY");
        std::env::remove_var("HIVE_INSECURE_SSL");
    }
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture_without_binaries(&paths, "rg", "14.1.0", &["bin/rg"]);
    let existing_binary = paths.package_store.join("rg/14.1.0/bin/rg");
    write_file_with_mode(&existing_binary, b"existing-binary", 0o755);
    fs::create_dir_all(paths.shim_dir.parent().unwrap()).unwrap();
    fs::write(&paths.shim_dir, "blocker").unwrap();

    let prompts = tests_support::ScriptedInstallPrompts::new(&["1"]);
    let error = app::install_package_with_prompts(&paths, "rg", &prompts).unwrap_err();

    assert!(error.contains("failed to create"));
    assert_eq!(fs::read(&existing_binary).unwrap(), b"existing-binary");
    assert!(paths.package_store.join("rg/14.1.0.install-backup").symlink_metadata().is_err());
}

#[test]
fn install_command_uses_binary_basenames_for_shim_names() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "rg",
        &["14.1.0"],
        "14.1.0",
        &["bin/rg", "bin/rga"],
    );

    let cli = Cli::try_parse_from(["hive", "use", "rg", "14.1.0"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert!(paths.shim_dir.join("rg").exists());
    assert!(paths.shim_dir.join("rga").exists());
    assert!(!paths.shim_dir.join("bin/rg").exists());
    assert_eq!(
        fs::read_link(paths.shim_dir.join("rg")).unwrap(),
        paths.package_store.join("rg/current/bin/rg")
    );
    assert_eq!(
        fs::read_link(paths.shim_dir.join("rga")).unwrap(),
        paths.package_store.join("rg/current/bin/rga")
    );
}

#[test]
fn install_command_does_not_switch_current_when_activation_fails() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture(&paths, "rg", "14.1.0");

    fs::create_dir_all(paths.shim_dir.parent().unwrap()).unwrap();
    fs::write(&paths.shim_dir, "blocker").unwrap();

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    let error = app::run_with_paths(cli, paths.clone()).unwrap_err();

    assert!(error.contains("failed to create"));
    assert!(
        paths
            .package_store
            .join("rg/current")
            .symlink_metadata()
            .is_err()
    );
}

#[test]
fn install_command_rejects_duplicate_binary_basenames() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let archive_path = temp.path().join("rg.tar.gz");
    let source_dir = temp.path().join("source");
    fs::create_dir_all(source_dir.join("bin")).unwrap();
    fs::create_dir_all(source_dir.join("alt")).unwrap();
    let bin_rg = source_dir.join("bin/rg");
    let alt_rg = source_dir.join("alt/rg");
    fs::write(&bin_rg, "stub-binary").unwrap();
    fs::write(&alt_rg, "stub-binary").unwrap();
    tests_support::write_tar_gz_files(
        &archive_path,
        &[(bin_rg.as_path(), "bin/rg"), (alt_rg.as_path(), "alt/rg")],
    );

    let bytes = fs::read(&archive_path).unwrap();
    let checksum = format!("sha256:{:x}", Sha256::digest(bytes));
    tests_support::write_manifest_with_binaries_with_archive(
        &paths,
        "rg",
        "14.1.0",
        &archive_path,
        &checksum,
        &["bin/rg", "alt/rg"],
        "tar.gz",
    );

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    let error = app::run_with_paths(cli, paths.clone()).unwrap_err();

    assert!(error.contains("duplicate shim name"));
    assert!(
        paths
            .package_store
            .join("rg/current")
            .symlink_metadata()
            .is_err()
    );
}

#[test]
fn install_command_rolls_back_shims_when_state_save_fails_after_activation() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture(&paths, "rg", "14.1.0");
    fs::create_dir_all(&paths.state_dir).unwrap();
    let state_file = paths.state_dir.join("rg.json");
    fs::write(&state_file, r#"{"name":"rg","versions":[],"active":null}"#).unwrap();
    let mut permissions = fs::metadata(&state_file).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o444);
    }
    fs::set_permissions(&state_file, permissions).unwrap();

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    let error = app::run_with_paths(cli, paths.clone()).unwrap_err();

    assert!(error.contains("failed to write"));
    assert!(
        paths
            .package_store
            .join("rg/current")
            .symlink_metadata()
            .is_err()
    );
    assert!(paths.shim_dir.join("rg").symlink_metadata().is_err());
}

#[test]
fn install_fails_on_checksum_mismatch() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_bad_checksum_fixture(&paths, "rg", "14.1.0");

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    let error = app::run_with_paths(cli, paths).unwrap_err();

    assert!(error.contains("checksum mismatch"));
}

#[test]
fn install_fails_when_declared_binary_is_missing() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_missing_binary_fixture(&paths, "rg", "14.1.0");

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    let error = app::run_with_paths(cli, paths.clone()).unwrap_err();

    assert!(error.contains("declared binary missing"));
    assert!(!paths.package_store.join("rg/14.1.0").exists());
}
