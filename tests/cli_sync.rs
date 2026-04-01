#[path = "support/mod.rs"]
mod tests_support;

use clap::Parser;
use hive::{
    app,
    cli::{Cli, Commands},
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
fn run_capture_routes_sync_without_printing_output() {
    let temp = tempdir().unwrap();
    let paths = tests_support::fixture_paths(temp.path());
    let cli = Cli::try_parse_from(["hive", "sync", "BurntSushi/ripgrep"]).unwrap();

    let error = app::run_capture(cli, paths).unwrap_err();
    assert!(error.contains("sync is not implemented"));
}
