#[path = "support/mod.rs"]
mod tests_support;

use clap::Parser;
use hive::{app, cli::Cli};
use std::fs;
use tempfile::tempdir;

#[test]
fn uninstall_refuses_to_remove_active_version_without_force() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package(&paths, "rg", &["14.1.0"], "14.1.0");

    let cli = Cli::try_parse_from(["hive", "uninstall", "rg", "14.1.0"]).unwrap();
    let error = app::run_with_paths(cli, paths.clone()).unwrap_err();

    assert!(error.contains("active version"));
    assert!(paths.package_store.join("rg/14.1.0").exists());
}

#[test]
fn uninstall_removes_non_active_version() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package(&paths, "rg", &["14.0.0", "14.1.0"], "14.1.0");

    let cli = Cli::try_parse_from(["hive", "uninstall", "rg", "14.0.0"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert!(!paths.package_store.join("rg/14.0.0").exists());
}

#[test]
fn forced_uninstall_removes_active_exported_shims() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "ripgrep",
        &["14.1.0"],
        "14.1.0",
        &["rg", "rga"],
    );

    std::os::unix::fs::symlink(
        paths.package_store.join("ripgrep/14.1.0"),
        paths.package_store.join("ripgrep/current"),
    )
    .unwrap();
    fs::create_dir_all(&paths.shim_dir).unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("ripgrep/current/rg"),
        paths.shim_dir.join("rg"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("ripgrep/current/rga"),
        paths.shim_dir.join("rga"),
    )
    .unwrap();
    assert!(paths.shim_dir.join("rg").symlink_metadata().is_ok());
    assert!(paths.shim_dir.join("rga").symlink_metadata().is_ok());

    let cli = Cli::try_parse_from(["hive", "uninstall", "ripgrep", "14.1.0", "--force"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert!(!paths.package_store.join("ripgrep/14.1.0").exists());
    assert!(
        paths
            .package_store
            .join("ripgrep/current")
            .symlink_metadata()
            .is_err()
    );
    assert!(paths.shim_dir.join("rg").symlink_metadata().is_err());
    assert!(paths.shim_dir.join("rga").symlink_metadata().is_err());
}

#[test]
fn forced_uninstall_removes_current_symlink_and_nested_binary_shims() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "helix",
        &["25.07.1"],
        "25.07.1",
        &["bin/hx"],
    );

    std::os::unix::fs::symlink(
        paths.package_store.join("helix/25.07.1"),
        paths.package_store.join("helix/current"),
    )
    .unwrap();
    fs::create_dir_all(&paths.shim_dir).unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("helix/current/bin/hx"),
        paths.shim_dir.join("hx"),
    )
    .unwrap();

    let cli = Cli::try_parse_from(["hive", "uninstall", "helix", "25.07.1", "--force"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert!(
        paths
            .package_store
            .join("helix/current")
            .symlink_metadata()
            .is_err()
    );
    assert!(paths.shim_dir.join("hx").symlink_metadata().is_err());
}
