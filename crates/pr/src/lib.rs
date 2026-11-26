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
            eprintln!("belt-pr: {e}");
            eprintln!("run `belt-pr --help` for usage.");
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
    let raw = match gh::fetch_pr_json(p.target.as_deref(), p.repo.as_deref(), c) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} {e}", c.red("error:"));
            return 1;
        }
    };

    if p.raw_json {
        if let Err(e) = writeln!(out, "{}", raw.trim_end()) {
            eprintln!("{} failed to write output: {e}", c.red("error:"));
            return 1;
        }
        return 0;
    }

    let mut pr = match PullRequest::from_json_str(&raw) {
        Ok(pr) => pr,
        Err(e) => {
            eprintln!("{} {e}", c.red("error:"));
            return 1;
        }
    };

    // The HUD never lists files, so skip the extra paginated request there.
    if p.all_files && !p.hud && pr.changed_files > pr.files.len() as u64 {
        match gh::fetch_all_pr_files(&pr.url) {
            Ok(files) if !files.is_empty() => pr.files = files,
            Ok(_) => {}
            Err(e) => eprintln!(
                "{} failed to fetch all changed files: {e}",
                c.yellow("warning:")
            ),
        }
    }

    if p.hud {
        match gh::fetch_review_threads(&pr.url) {
            Ok(counts) => pr.review_threads = Some(counts),
            Err(e) => eprintln!(
                "{} failed to fetch review threads: {e}",
                c.yellow("warning:")
            ),
        }
    }

    let opts = RenderOpts {
        width: terminal_columns(),
        compact: p.compact,
        hud: p.hud,
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
