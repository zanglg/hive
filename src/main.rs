use clap::Parser;
use hive::{app, cli::Cli};

fn main() {
    let cli = Cli::parse();
    if let Err(error) = app::run(cli) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
