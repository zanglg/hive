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
    let current = temp.path().join("pkgs/rg/current");
    fs::create_dir_all(&install_root).unwrap();
    let binary = install_root.join("rg");
    fs::write(&binary, "stub").unwrap();
    std::os::unix::fs::symlink(&install_root, &current).unwrap();

    let shim_dir = temp.path().join("bin/hive");
    activate_version(&shim_dir, &[("rg".into(), binary.clone())]).unwrap();

    let link = shim_dir.join("rg");
    assert!(link.exists());
    assert_eq!(fs::read_link(link).unwrap(), binary);
}

#[test]
fn use_command_switches_current_version_and_keeps_shims_on_current() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "rg",
        &["14.0.0", "14.1.0"],
        "14.0.0",
        &["bin/rg"],
    );

    let cli = Cli::try_parse_from(["hive", "use", "rg", "14.1.0"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert_eq!(
        fs::read_link(paths.package_store.join("rg/current")).unwrap(),
        paths.package_store.join("rg/14.1.0")
    );
    assert_eq!(
        fs::read_link(paths.shim_dir.join("rg")).unwrap(),
        paths.package_store.join("rg/current/bin/rg")
    );
}

#[test]
fn use_command_removes_stale_shims_when_switching_to_fewer_binaries() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "rg",
        &["14.0.0", "14.1.0"],
        "14.0.0",
        &["bin/rg"],
    );
    fs::write(paths.package_store.join("rg/14.0.0/rga"), "stale").unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/14.0.0"),
        paths.package_store.join("rg/current"),
    )
    .unwrap();
    fs::create_dir_all(&paths.shim_dir).unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/current/rg"),
        paths.shim_dir.join("rg"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/current/rga"),
        paths.shim_dir.join("rga"),
    )
    .unwrap();

    let cli = Cli::try_parse_from(["hive", "use", "rg", "14.1.0"]).unwrap();
    app::run_with_paths(cli, paths.clone()).unwrap();

    assert_eq!(
        fs::read_link(paths.package_store.join("rg/current")).unwrap(),
        paths.package_store.join("rg/14.1.0")
    );
    assert!(paths.shim_dir.join("rg").symlink_metadata().is_ok());
    assert!(paths.shim_dir.join("rga").symlink_metadata().is_err());
}

#[test]
fn use_command_does_not_switch_current_when_activation_fails() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "rg",
        &["14.0.0", "14.1.0"],
        "14.0.0",
        &["bin/rg"],
    );
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/14.0.0"),
        paths.package_store.join("rg/current"),
    )
    .unwrap();
    fs::create_dir_all(paths.shim_dir.parent().unwrap()).unwrap();
    fs::write(&paths.shim_dir, "blocker").unwrap();

    let cli = Cli::try_parse_from(["hive", "use", "rg", "14.1.0"]).unwrap();
    let error = app::run_with_paths(cli, paths.clone()).unwrap_err();

    assert!(error.contains("failed to create"));
    assert_eq!(
        fs::read_link(paths.package_store.join("rg/current")).unwrap(),
        paths.package_store.join("rg/14.0.0")
    );
    let store = StateStore::new(paths.state_dir.clone());
    let package = store.load_package("rg").unwrap().unwrap();
    assert_eq!(package.active.as_deref(), Some("14.0.0"));
}

#[test]
fn use_command_restores_previous_activation_when_state_update_fails_after_shim_mutation() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "rg",
        &["14.0.0", "14.1.0"],
        "14.0.0",
        &["bin/rg"],
    );
    let state_file = paths.state_dir.join("rg.json");
    fs::write(
        &state_file,
        r#"{
  "name": "rg",
  "versions": [
    "14.0.0"
  ],
  "active": "14.0.0"
}"#,
    )
    .unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/14.0.0"),
        paths.package_store.join("rg/current"),
    )
    .unwrap();
    fs::create_dir_all(&paths.shim_dir).unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/current/rg"),
        paths.shim_dir.join("rg"),
    )
    .unwrap();

    let cli = Cli::try_parse_from(["hive", "use", "rg", "14.1.0"]).unwrap();
    let error = app::run_with_paths(cli, paths.clone()).unwrap_err();

    assert!(error.contains("does not have version"));
    assert_eq!(
        fs::read_link(paths.package_store.join("rg/current")).unwrap(),
        paths.package_store.join("rg/14.0.0")
    );
    assert_eq!(
        fs::read_link(paths.shim_dir.join("rg")).unwrap(),
        paths.package_store.join("rg/current/rg")
    );
}

#[test]
fn which_reports_active_binary_path_through_current() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "rg",
        &["14.1.0"],
        "14.1.0",
        &["bin/rg"],
    );
    fs::create_dir_all(&paths.shim_dir).unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/current/bin/rg"),
        paths.shim_dir.join("rg"),
    )
    .unwrap();

    let cli = Cli::try_parse_from(["hive", "which", "rg"]).unwrap();
    let output = app::run_capture(cli, paths).unwrap();
    assert!(output.contains("rg/current/bin/rg"));
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
    tests_support::seed_installed_package_with_binaries(
        &paths,
        "rg",
        &["14.1.0"],
        "14.1.0",
        &["bin/rg"],
    );
    fs::create_dir_all(&paths.shim_dir).unwrap();
    std::os::unix::fs::symlink(
        paths.package_store.join("rg/current/bin/rg"),
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
