use std::io::{self, BufRead, Write};
use std::process::{Command, ExitCode};

use belt::{find_tool, handle, resolve_bin, TOOLS};

fn usage() -> String {
    let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    format!(
        "belt {}\n\nUsage:\n  belt <tool> [args...]   run a belt tool\n  belt mcp                serve all tools as a stdio MCP server\n\nTools: {}",
        env!("CARGO_PKG_VERSION"),
        names.join(", ")
    )
}

fn serve() -> ExitCode {
    let stdout = io::stdout();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else {
            return ExitCode::FAILURE;
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(&line, &resolve_bin) {
            let mut out = stdout.lock();
            if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("mcp") => serve(),
        None | Some("-h" | "--help") => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("belt {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(name) => {
            let Some(tool) = find_tool(name) else {
                eprintln!("belt: unknown tool '{name}'\n\n{}", usage());
                return ExitCode::from(2);
            };
            let bin = resolve_bin(tool.name);
            match Command::new(&bin).args(args).status() {
                Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
                Err(err) => {
                    eprintln!("belt: failed to run {}: {err}", bin.display());
                    ExitCode::FAILURE
                }
            }
        }
    }
}
