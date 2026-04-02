use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "hive",
    about = "Manage portable CLI tool installs from local manifests",
    long_about = "Hive manages portable CLI tool installs from local manifests.\n\nUse `hive install` to install a package version described by a manifest, `hive sync` to refresh manifests from GitHub releases, and `hive use` to switch which installed version is active."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(
        about = "Install a package from a local manifest",
        long_about = "Install a package from a local manifest.\n\nHive resolves the named manifest, downloads the artifact for the current platform, verifies its checksum, extracts it into the package store, and activates it if no version is active yet.\n\nExamples:\n  hive install rg"
    )]
    Install {
        #[arg(value_name = "PACKAGE", help = "Manifest name to install")]
        package: String,
    },
    #[command(
        about = "List installed packages and their active versions",
        long_about = "List installed packages and their active versions.\n\nThe active version is marked with `*` in the output.\n\nExamples:\n  hive list"
    )]
    List,
    #[command(
        about = "Create or update a manifest from a GitHub release",
        long_about = "Create or update a manifest from the latest qualifying GitHub release.\n\nHive fetches release metadata, maps supported platform assets, computes checksums, and writes the manifest into the local manifest directory.\n\nExamples:\n  hive sync BurntSushi/ripgrep"
    )]
    Sync {
        #[arg(value_name = "REPO", help = "GitHub repository in owner/name form")]
        repo: String,
    },
    #[command(
        about = "Switch the active installed version for a package",
        long_about = "Switch the active installed version for a package.\n\nHive updates the package `current` symlink and rewrites managed shims so the selected version becomes active.\n\nExamples:\n  hive use rg 14.1.0"
    )]
    Use {
        #[arg(value_name = "PACKAGE", help = "Installed package to activate")]
        package: String,
        #[arg(value_name = "VERSION", help = "Installed version to activate")]
        version: String,
    },
    #[command(
        about = "Remove an installed package version",
        long_about = "Remove an installed package version.\n\nHive refuses to remove the active version unless `--force` is provided.\n\nExamples:\n  hive uninstall rg 14.1.0\n  hive uninstall rg 14.1.0 --force"
    )]
    Uninstall {
        #[arg(value_name = "PACKAGE", help = "Installed package to remove")]
        package: String,
        #[arg(value_name = "VERSION", help = "Installed version to remove")]
        version: String,
        #[arg(long, help = "Remove the active version as well")]
        force: bool,
    },
    #[command(
        about = "Show the active binary path for a package",
        long_about = "Show the active binary path for a package.\n\nThis prints the path for single-binary packages and errors if the package exports more than one binary.\n\nExamples:\n  hive which helix"
    )]
    Which {
        #[arg(value_name = "PACKAGE", help = "Installed package to inspect")]
        package: String,
    },
}
