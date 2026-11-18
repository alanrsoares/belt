//! `pr` — terminal-friendly, visually compelling GitHub pull request overview.

mod cli;
mod gh;
mod model;
mod render;

use std::io::{self, Write};

use local_common::{terminal_columns, Glyphs};

use cli::{Action, Parsed};
use model::{now_epoch_secs, PullRequest};
use render::{render_pr, RenderOpts};

/// Main runner invoked by `main.rs`.
pub fn run<I: IntoIterator<Item = String>>(args: I) -> i32 {
    let p = match cli::parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pr: {e}");
            eprintln!("run `pr --help` for usage.");
            return 1;
        }
    };

    let mut out = io::stdout();
    let c = p.flags.resolve_colour(&out);

    match p.action {
        Action::Help => {
            print!("{}", cli::HELP);
            0
        }
        Action::Version => {
            println!("{}", cli::VERSION);
            0
        }
        Action::Web => match gh::open_pr_web(p.target.as_deref(), p.repo.as_deref()) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{} {e}", c.red("error:"));
                1
            }
        },
        Action::View => run_view(&p, &c, &mut out),
    }
}

fn run_view(p: &Parsed, c: &local_common::Colour, out: &mut impl Write) -> i32 {
    let raw = match gh::fetch_pr_json(p.target.as_deref(), p.repo.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} {e}", c.red("error:"));
            return 1;
        }
    };

    if p.raw_json {
        let _ = writeln!(out, "{}", raw.trim_end());
        return 0;
    }

    let pr = match PullRequest::from_json_str(&raw) {
        Ok(pr) => pr,
        Err(e) => {
            eprintln!("{} {e}", c.red("error:"));
            return 1;
        }
    };

    let opts = RenderOpts {
        width: terminal_columns(),
        compact: p.compact,
        all_files: p.all_files,
        full_body: p.full_body,
        all_checks: p.all_checks,
        now_secs: now_epoch_secs(),
        glyphs: Glyphs::for_stream(&io::stdout()),
    };

    if let Err(e) = render_pr(&pr, c, opts, out) {
        eprintln!("{} failed to write output: {e}", c.red("error:"));
        return 1;
    }

    0
}
