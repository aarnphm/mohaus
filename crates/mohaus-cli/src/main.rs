use std::process::ExitCode;

fn main() -> ExitCode {
    match mohaus_cli::run_from(std::env::args_os()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}
