//! `automata-cli`: export AUTOMATA games from the command line (see
//! [`automata_cli::cli`]).

use std::process::ExitCode;

fn main() -> ExitCode {
    match automata_cli::cli::run(std::env::args().skip(1), "automata-cli") {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
