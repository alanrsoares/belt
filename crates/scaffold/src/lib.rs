//! `scaffold` — lay down a ready-to-run project the way you already do.
//!
//! A thin orchestration layer over [`templates`]. The binary (`main.rs`) is a
//! one-line call into [`run`], which parses the CLI, resolves colours, and
//! dispatches. All the logic lives here so it is testable in-process with a
//! `Vec<String>` of args and a temp directory.

mod cli;
mod log;
mod templates;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use local_common::{tool_config_dir, Colour};

use crate::templates::{File, Lang};
use cli::{Action, Parsed};

/// Parse, dispatch, and execute the given CLI args. Returns the process exit
/// code (0 on success, non-zero on a handled error).
pub fn run<I: IntoIterator<Item = String>>(args: I) -> i32 {
    let p = match cli::parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("scaffold: {e}");
            eprintln!("run `scaffold --help` for usage.");
            return 1;
        }
    };

    let mut out = std::io::stdout();
    let c = p.flags.resolve_colour(&out);

    match p.action {
        Action::Version => {
            print!("{}", cli::VERSION);
            0
        }
        Action::Help => {
            print!("{}", cli::HELP);
            0
        }
        Action::List => {
            list_projects(&c, &mut out);
            0
        }
        Action::Scaffold => run_scaffold(&p, &c, &mut out),
    }
}

/// List the available languages with their default names and v1 status.
fn list_projects(c: &Colour, out: &mut impl Write) {
    let _ = writeln!(out, "Languages — run `scaffold <lang>` to scaffold one:");
    for lang in Lang::all() {
        let status = if lang.supported() {
            c.green("ready")
        } else {
            c.yellow("v2")
        };
        let padded_name = format!("{:<6}", lang);
        let name = if lang.supported() {
            c.cyan(padded_name)
        } else {
            c.yellow(padded_name)
        };
        let padded_label = format!("{:<28}", lang.label());
        let _ = writeln!(out, "    {name}    {}    {status}", c.bold(padded_label));
    }
}

/// Execute a `scaffold <lang>` invocation.
fn run_scaffold(p: &Parsed, c: &Colour, out: &mut impl Write) -> i32 {
    let lang = match p.lang {
        Some(l) => l,
        // `parse` already guarantees a lang for the Scaffold action; this is a
        // belt-and-braces guard.
        None => {
            let _ = writeln!(out, "{} missing <LANG>.", c.red("error:"));
            return 1;
        }
    };

    let raw_name = p
        .name
        .clone()
        .unwrap_or_else(|| lang.default_name().to_string());

    let trimmed = raw_name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || raw_name.contains('/')
        || raw_name.contains('\\')
    {
        let _ = writeln!(out, "{} invalid project name: {raw_name}", c.red("error:"));
        return 1;
    }

    let name = templates::sanitize_name(&raw_name);

    // V1 does not yet wire a shell-out for .NET (`dotnet new`).
    if !lang.supported() {
        let _ = writeln!(out, "{} not wired in v1 yet.", c.yellow(format!("{lang}")));
        let _ = writeln!(
            out,
            "Run `dotnet new webapi -minimal -n {name}` for now (planned for v2)."
        );
        return 0;
    }

    let target: PathBuf = p.path.join(&name);
    if dir_has_files(&target) && !p.force {
        let _ = writeln!(
            out,
            "{} target already exists and is not empty: {}",
            c.red("error:"),
            target.display()
        );
        let _ = writeln!(
            out,
            "  pass --force to overwrite, or pick a --name / --path."
        );
        return 1;
    }

    let files = lang.render(&name);

    if p.dry_run {
        let _ = writeln!(
            out,
            "{} {} {} would create {} file(s) at {}",
            c.cyan("dry-run:"),
            c.cyan(format!("scaffold {lang}")),
            c.bold(name.clone()),
            files.len(),
            target.display()
        );
        for f in &files {
            let _ = writeln!(out, "    + {}", c.cyan(&f.path));
        }
        return 0;
    }

    match write_files(&target, &files) {
        Ok(_) => {
            let _ = writeln!(
                out,
                "{} {} {}",
                c.green("✓"),
                c.cyan(format!("scaffold {lang}")),
                target.display()
            );
            for f in &files {
                let _ = writeln!(out, "    + {}", f.path);
            }
            if !p.no_log {
                log_scaffold(p, lang, &name, &target);
            }
            let _ = writeln!(
                out,
                "\n{} cd {} && {}",
                c.cyan("next:"),
                c.bold(name),
                lang.next_hint()
            );
            0
        }
        Err(e) => {
            let _ = writeln!(out, "{} {}", c.red("error:"), e);
            1
        }
    }
}

/// Append a best-effort record of the scaffold to the recent log. Any filesystem
/// problem is swallowed so logging can never fail the scaffold itself.
fn log_scaffold(p: &Parsed, lang: Lang, name: &str, target: &Path) {
    if p.no_log {
        return;
    }
    if let Some(cfg) = tool_config_dir("scaffold") {
        let line = log::record_line(&unix_secs(), lang, name, target);
        let _ = log::append(&cfg, &line);
    }
}

/// Whether `target` exists and is a non-empty directory, in which case a plain
/// scaffold would clobber it. A missing target or an empty directory is fine.
fn dir_has_files(target: &Path) -> bool {
    let meta = match fs::metadata(target) {
        Ok(m) => m,
        // Missing target — clean to create.
        Err(_) => return false,
    };
    if meta.is_dir() {
        match fs::read_dir(target) {
            Ok(mut entries) => entries.any(|e| e.is_ok()),
            // An unreadable directory is treated as a conflict (safer default).
            Err(_) => true,
        }
    } else {
        // Exists but is not a directory (e.g. a file of the same name) — conflict.
        true
    }
}

/// Create parent directories as needed and write each file verbatim. Returns the
/// number of files written.
fn write_files(target: &Path, files: &[File]) -> std::io::Result<usize> {
    let mut n = 0_usize;
    for f in files {
        let full = target.join(&f.path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full, &f.contents)?;
        n += 1;
    }
    Ok(n)
}

/// Seconds since the Unix epoch, as a string. Falls back to `"0"` if the system
/// clock is somehow before the epoch.
fn unix_secs() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use local_common::strip_ansi;

    #[test]
    fn run_help_and_version_actions_succeed() {
        assert_eq!(run(vec!["--help".to_string()]), 0);
        assert_eq!(run(vec!["-h".to_string()]), 0);
        assert_eq!(run(vec!["--version".to_string()]), 0);
        assert_eq!(run(vec!["-V".to_string()]), 0);
        assert_eq!(run(vec!["--list".to_string()]), 0);
        assert_eq!(run(vec!["-L".to_string()]), 0);
    }

    #[test]
    fn run_missing_args_returns_non_zero() {
        assert_eq!(run(Vec::<String>::new()), 1);
        assert_eq!(run(vec!["--invalid-flag".to_string()]), 1);
    }

    #[test]
    fn run_dry_run_scaffolding() {
        let tmp = std::env::temp_dir().join("scaffold-test-dry-run");
        let _ = fs::remove_dir_all(&tmp);
        let code = run(vec![
            "rust".to_string(),
            "--name".to_string(),
            "test-app".to_string(),
            "--path".to_string(),
            tmp.to_string_lossy().to_string(),
            "--dry-run".to_string(),
        ]);
        assert_eq!(code, 0);
        assert!(!tmp.exists());
    }

    #[test]
    fn run_rejects_path_traversal_and_invalid_names() {
        let tmp = std::env::temp_dir().join("scaffold-test-traversal");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        for bad in ["../evil", r"..\evil", "/tmp/evil", ".", "..", "a/b"] {
            let mut out = Vec::new();
            let p = Parsed {
                action: Action::Scaffold,
                lang: Some(Lang::Ts),
                name: Some(bad.to_string()),
                path: tmp.clone(),
                no_log: true,
                ..Default::default()
            };
            let code = run_scaffold(&p, &Colour::new(false), &mut out);
            assert_eq!(code, 1, "expected failure for name {bad:?}");
            let msg = String::from_utf8(out).unwrap();
            assert!(
                msg.contains("invalid project name:"),
                "unexpected msg: {msg}"
            );
        }

        // Also ensure spaces in project name create sanitized directory matching rendered package name
        let mut out = Vec::new();
        let p = Parsed {
            action: Action::Scaffold,
            lang: Some(Lang::Ts),
            name: Some("my cool app".to_string()),
            path: tmp.clone(),
            no_log: true,
            ..Default::default()
        };
        assert_eq!(run_scaffold(&p, &Colour::new(false), &mut out), 0);
        assert!(tmp.join("my-cool-app").join("package.json").is_file());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn list_projects_aligns_columns_with_ansi_colours() {
        let mut out_col = Vec::new();
        let mut out_plain = Vec::new();
        list_projects(&Colour::new(true), &mut out_col);
        list_projects(&Colour::new(false), &mut out_plain);

        let text_col = strip_ansi(&String::from_utf8(out_col).unwrap());
        let text_plain = String::from_utf8(out_plain).unwrap();
        assert_eq!(text_col, text_plain);

        let lines: Vec<&str> = text_col.lines().skip(1).collect();
        assert_eq!(lines.len(), Lang::all().len());

        // Every row should have status column starting at the exact same column offset
        let status_cols: Vec<usize> = lines
            .iter()
            .map(|l| {
                l.rfind("ready")
                    .or_else(|| l.rfind("v2"))
                    .expect("missing status")
            })
            .collect();
        for col in &status_cols {
            assert_eq!(
                *col,
                status_cols[0],
                "misaligned status columns in:\n{}",
                lines.join("\n")
            );
        }
    }

    #[test]
    fn run_actual_scaffolding_and_conflict_handling() {
        let tmp = std::env::temp_dir().join("scaffold-test-actual");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let target = tmp.join("my-test-proj");

        // 1. Initial scaffold succeeds
        let code = run(vec![
            "ts".to_string(),
            "--name".to_string(),
            "my-test-proj".to_string(),
            "--path".to_string(),
            tmp.to_string_lossy().to_string(),
            "--no-log".to_string(),
        ]);
        assert_eq!(code, 0);
        assert!(target.join("package.json").is_file());
        assert!(target.join("index.ts").is_file());

        // 2. Conflict without --force fails
        let conflict_code = run(vec![
            "ts".to_string(),
            "--name".to_string(),
            "my-test-proj".to_string(),
            "--path".to_string(),
            tmp.to_string_lossy().to_string(),
            "--no-log".to_string(),
        ]);
        assert_eq!(conflict_code, 1);

        // 3. Re-scaffold with --force succeeds
        let force_code = run(vec![
            "ts".to_string(),
            "--name".to_string(),
            "my-test-proj".to_string(),
            "--path".to_string(),
            tmp.to_string_lossy().to_string(),
            "--force".to_string(),
            "--no-log".to_string(),
        ]);
        assert_eq!(force_code, 0);

        let _ = fs::remove_dir_all(&tmp);
    }
}
