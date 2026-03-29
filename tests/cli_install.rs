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
fn install_command_resolves_manifest_and_activates_first_version() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_install_fixture(&paths, "rg", "14.1.0");

    let cli = Cli::try_parse_from(["hive", "install", "rg"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert!(paths.package_store.join("rg/14.1.0/rg").exists());
    assert_eq!(
        fs::read_link(paths.shim_dir.join("rg")).unwrap(),
        paths.package_store.join("rg/14.1.0/rg")
    );
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
