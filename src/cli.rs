use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "hive")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Install {
        package: String,
    },
    List,
    Use {
        package: String,
        version: String,
    },
    Uninstall {
        package: String,
        version: String,
        #[arg(long)]
        force: bool,
    },
    Which {
        package: String,
    },
}
