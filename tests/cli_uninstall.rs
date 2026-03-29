#[path = "support/mod.rs"]
mod tests_support;

use clap::Parser;
use hive::{app, cli::Cli};
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
