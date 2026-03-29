#[path = "support/mod.rs"]
mod tests_support;

use clap::Parser;
use hive::{
    activation::activate_version,
    app,
    cli::Cli,
    state::{InstalledPackage, StateStore},
};
use std::fs;
use tempfile::tempdir;

#[test]
fn saves_and_loads_active_package_version() {
    let temp = tempdir().unwrap();
    let store = StateStore::new(temp.path().join("state"));

    store
        .save_package(&InstalledPackage {
            name: "rg".into(),
            versions: vec!["14.1.0".into(), "14.0.0".into()],
            active: Some("14.1.0".into()),
        })
        .unwrap();

    let package = store.load_package("rg").unwrap().unwrap();
    assert_eq!(package.active.as_deref(), Some("14.1.0"));
    assert_eq!(package.versions.len(), 2);
}

#[test]
fn creates_symlink_for_active_binary_in_hive_shim_dir() {
    let temp = tempdir().unwrap();
    let install_root = temp.path().join("pkgs/rg/14.1.0");
    let binary = install_root.join("rg");
    fs::create_dir_all(&install_root).unwrap();
    fs::write(&binary, "stub").unwrap();

    let shim_dir = temp.path().join("bin/hive");
    activate_version(&shim_dir, &[("rg".into(), binary.clone())]).unwrap();

    let link = shim_dir.join("rg");
    assert!(link.exists());
    assert_eq!(fs::read_link(link).unwrap(), binary);
}

#[test]
fn use_command_switches_active_version() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package(&paths, "rg", &["14.0.0", "14.1.0"], "14.0.0");

    let cli = Cli::try_parse_from(["hive", "use", "rg", "14.1.0"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert_eq!(
        fs::read_link(paths.shim_dir.join("rg")).unwrap(),
        paths.package_store.join("rg/14.1.0/rg")
    );
}

#[test]
fn which_reports_active_binary_path() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package(&paths, "rg", &["14.1.0"], "14.1.0");
    fs::create_dir_all(&paths.shim_dir).unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/14.1.0/rg"),
        paths.shim_dir.join("rg"),
    )
    .unwrap();

    let cli = Cli::try_parse_from(["hive", "which", "rg"]).unwrap();
    let output = app::run_capture(cli, paths).unwrap();
    assert!(output.contains("14.1.0/rg"));
}

#[test]
fn list_reports_installed_versions_and_marks_active_one() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package(&paths, "rg", &["14.0.0", "14.1.0"], "14.1.0");

    let cli = Cli::try_parse_from(["hive", "list"]).unwrap();
    let output = app::run_capture(cli, paths).unwrap();
    assert!(output.contains("rg 14.0.0"));
    assert!(output.contains("rg 14.1.0 *"));
}

#[test]
fn all_v1_commands_complete_without_placeholder_errors() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package(&paths, "rg", &["14.1.0"], "14.1.0");
    fs::create_dir_all(&paths.shim_dir).unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/14.1.0/rg"),
        paths.shim_dir.join("rg"),
    )
    .unwrap();

    let list_cli = Cli::try_parse_from(["hive", "list"]).unwrap();
    assert!(app::run_capture(list_cli, paths.clone()).is_ok());

    let which_cli = Cli::try_parse_from(["hive", "which", "rg"]).unwrap();
    assert!(app::run_capture(which_cli, paths.clone()).is_ok());

    let use_cli = Cli::try_parse_from(["hive", "use", "rg", "14.1.0"]).unwrap();
    assert!(app::run_capture(use_cli, paths.clone()).is_ok());

    let uninstall_cli =
        Cli::try_parse_from(["hive", "uninstall", "rg", "14.1.0", "--force"]).unwrap();
    assert!(app::run_capture(uninstall_cli, paths).is_ok());
}
