#[path = "support/mod.rs"]
mod tests_support;

use clap::Parser;
use hive::{
    app,
    cli::{Cli, Commands},
    config::HivePaths,
    installer::{ArchiveKind, Installer},
    manifest::{Manifest, ManifestRepository},
    platform::Platform,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

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
fn renders_manifest_with_github_source_block_before_platforms() {
    let manifest = tests_support::manifest_with_github_source(
        "rg",
        "14.1.0",
        "BurntSushi/ripgrep",
        "stable",
    );
    let rendered = manifest.to_toml().unwrap();

    let source_block = "[source.github]\nrepo = \"BurntSushi/ripgrep\"\nchannel = \"stable\"";
    let platform_block = format!("[platform.{}]", tests_support::current_platform_key());

    let source_index = rendered.find(source_block).unwrap();
    let platform_index = rendered.find(&platform_block).unwrap();

    assert!(source_index < platform_index);
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
fn install_keeps_single_wrapper_directory_unchanged_when_declared_binaries_only_exist_via_symlink() {
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
    assert!(paths.package_store.join("rg/current").symlink_metadata().is_err());
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
        &[
            (bin_rg.as_path(), "bin/rg"),
            (alt_rg.as_path(), "alt/rg"),
        ],
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
    assert!(paths.package_store.join("rg/current").symlink_metadata().is_err());
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
    assert!(paths.package_store.join("rg/current").symlink_metadata().is_err());
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
