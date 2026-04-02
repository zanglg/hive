use clap::CommandFactory;
use hive::cli::Cli;

fn render_help(mut command: clap::Command) -> String {
    let mut buffer = Vec::new();
    command.write_long_help(&mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

#[test]
fn root_help_lists_each_command_with_summary() {
    let help = render_help(Cli::command());

    assert!(help.contains("install    Install a package from a local manifest"));
    assert!(help.contains("list       List installed packages and their active versions"));
    assert!(help.contains("sync       Create or update a manifest from a GitHub release"));
    assert!(help.contains("use        Switch the active installed version for a package"));
    assert!(help.contains("uninstall  Remove an installed package version"));
    assert!(help.contains("which      Show the active binary path for a package"));
}

#[test]
fn subcommand_help_includes_examples_and_argument_descriptions() {
    let install_help = render_help(
        Cli::command()
            .find_subcommand_mut("install")
            .unwrap()
            .clone(),
    );
    assert!(install_help.contains("Install a package from a local manifest."));
    assert!(install_help.contains("Examples:"));
    assert!(install_help.contains("hive install rg"));
    assert!(install_help.contains("<PACKAGE>"));
    assert!(install_help.contains("Manifest name to install"));

    let sync_help = render_help(Cli::command().find_subcommand_mut("sync").unwrap().clone());
    assert!(
        sync_help
            .contains("Create or update a manifest from the latest qualifying GitHub release.")
    );
    assert!(sync_help.contains("hive sync BurntSushi/ripgrep"));
    assert!(sync_help.contains("<REPO>"));
    assert!(sync_help.contains("GitHub repository in owner/name form"));

    let use_help = render_help(Cli::command().find_subcommand_mut("use").unwrap().clone());
    assert!(use_help.contains("Switch the active installed version for a package."));
    assert!(use_help.contains("hive use rg 14.1.0"));
    assert!(use_help.contains("<VERSION>"));
    assert!(use_help.contains("Installed version to activate"));

    let uninstall_help = render_help(
        Cli::command()
            .find_subcommand_mut("uninstall")
            .unwrap()
            .clone(),
    );
    assert!(uninstall_help.contains("Remove an installed package version."));
    assert!(uninstall_help.contains("hive uninstall rg 14.1.0 --force"));
    assert!(uninstall_help.contains("--force"));
    assert!(uninstall_help.contains("Remove the active version as well"));

    let which_help = render_help(Cli::command().find_subcommand_mut("which").unwrap().clone());
    assert!(which_help.contains("Show the active binary path for a package."));
    assert!(which_help.contains("hive which helix"));
}
