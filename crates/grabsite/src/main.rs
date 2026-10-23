//! `grabsite` binary entrypoint.

use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut stdout = io::stdout();
    let code = grabsite::run(std::env::args().skip(1).collect(), &mut stdout).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        1
    });
    let _ = stdout.flush();
    ExitCode::from(code as u8)
}
