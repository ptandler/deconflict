mod cli;
mod cli_config;
mod commands;
mod interactive;
mod progress;

use clap::Parser;
use cli::Cli;

fn main() {
    if let Err(message) = commands::run(Cli::parse()) {
        eprintln!("{message}");
        std::process::exit(2);
    }
}
