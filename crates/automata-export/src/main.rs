//! `automata-export`: export AUTOMATA games from the command line (see
//! [`automata_export::cli`]).

use std::process::ExitCode;

fn main() -> ExitCode {
    match automata_export::cli::run(std::env::args().skip(1), "automata-export") {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
