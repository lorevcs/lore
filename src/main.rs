use std::process::ExitCode;

use clap::Parser;
use lore::cli::{run, Cli};

fn main() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("lore: {e}");
            return ExitCode::FAILURE;
        }
    };
    match run(Cli::parse(), &cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lore: {e}");
            ExitCode::FAILURE
        }
    }
}
