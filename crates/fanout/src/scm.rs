//! Git-aware change detection for monorepo package scoping (inspired by Turborepo SCM).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::workspace::WorkspacePkg;

const ROOT_CONFIG_FILES: &[&str] = &[
    "package.json",
    "fanout.json",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "yarn.lock",
    "package-lock.json",
    "tsconfig.json",
    "tsconfig.base.json",
    "turbo.json",
    "Cargo.toml",
    "Cargo.lock",
    "biome.json",
    ".oxlintrc.json",
    "lefthook.yml",
    ".gitignore",
];

/// Query git for files modified since a given git ref, plus staged/unstaged/untracked changes.
pub fn find_changed_files(root_dir: &Path, since_ref: &str) -> Result<Vec<PathBuf>, String> {
    if since_ref.starts_with('-') {
        return Err(format!("invalid git ref '{since_ref}'"));
    }

    let mut files = HashSet::new();

    // Working-tree state (staged, unstaged, untracked) in one call, started
    // now so it runs while the ref diff below does.
    let status = Command::new("git")
        .args(["status", "--porcelain", "-z", "--untracked-files=all"])
        .current_dir(root_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    // Commits diff against since_ref
    let diff_target = format!("{since_ref}...HEAD");
    let three_dot = Command::new("git")
        .args(["diff", "--name-only", "-z", &diff_target])
        .current_dir(root_dir)
        .output();

    match three_dot {
        Ok(output) if output.status.success() => {
            parse_nul_paths(&output.stdout, &mut files);
        }
        _ => {
            // Fallback to direct two-dot diff or single ref if 3-dot fails
            let fallback = Command::new("git")
                .args(["diff", "--name-only", "-z", since_ref])
                .current_dir(root_dir)
                .output()
                .map_err(|e| format!("failed to run git diff: {e}"))?;
            if fallback.status.success() {
                parse_nul_paths(&fallback.stdout, &mut files);
            } else {
                return Err(format!("unknown or invalid git ref '{since_ref}'"));
            }
        }
    }

    if let Ok(output) = status.and_then(|child| child.wait_with_output()) {
        if output.status.success() {
            parse_status_z(&output.stdout, &mut files);
        }
    }

    let mut result: Vec<PathBuf> = files.into_iter().collect();
    result.sort();
    Ok(result)
}

/// NUL-separated paths, as printed by `git diff --name-only -z`.
fn parse_nul_paths(stdout: &[u8], out: &mut HashSet<PathBuf>) {
    for path in stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        out.insert(PathBuf::from(String::from_utf8_lossy(path).as_ref()));
    }
}

/// `git status --porcelain -z`: `XY path` entries; a rename or copy is
/// followed by its source path, which counts as changed too.
fn parse_status_z(stdout: &[u8], out: &mut HashSet<PathBuf>) {
    let mut entries = stdout.split(|&b| b == 0).filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        let Some(path) = entry.get(3..) else { continue };
        out.insert(PathBuf::from(String::from_utf8_lossy(path).as_ref()));
        if entry[..2].iter().any(|&c| c == b'R' || c == b'C') {
            if let Some(source) = entries.next() {
                out.insert(PathBuf::from(String::from_utf8_lossy(source).as_ref()));
            }
        }
    }
}

/// Map modified files to workspace packages, including downstream dependents if requested.
pub fn resolve_affected_packages(
    pkgs: &[WorkspacePkg],
    changed_files: &[PathBuf],
    root_dir: &Path,
    include_dependents: bool,
) -> Vec<String> {
    if changed_files.is_empty() {
        return Vec::new();
    }

    // Check if any root manifest or global config changed
    for file in changed_files {
        let file_str = file.to_string_lossy();
        if ROOT_CONFIG_FILES.iter().any(|&cfg| file_str == cfg)
            || file_str.starts_with(".github/")
            || file_str.starts_with(".gitlab/")
        {
            return pkgs.iter().map(|p| p.name.clone()).collect();
        }
    }

    let mut directly_changed = HashSet::new();

    for pkg in pkgs {
        let rel_pkg_dir = match pkg.dir.strip_prefix(root_dir) {
            Ok(rel) => rel,
            Err(_) => Path::new(&pkg.name),
        };

        for file in changed_files {
            if file.starts_with(rel_pkg_dir) {
                directly_changed.insert(pkg.name.clone());
                break;
            }
        }
    }

    if !include_dependents {
        let mut list: Vec<String> = directly_changed.into_iter().collect();
        list.sort();
        return list;
    }

    // Build reverse dependency graph: dependency -> list of packages that depend on it
    let mut reverse_deps: HashMap<&str, Vec<&str>> = HashMap::new();
    for pkg in pkgs {
        for dep in &pkg.internal_deps {
            reverse_deps
                .entry(dep.as_str())
                .or_default()
                .push(&pkg.name);
        }
    }

    // Cascade changes downstream to all dependents (BFS)
    let mut affected: HashSet<String> = directly_changed.clone();
    let mut queue: Vec<String> = directly_changed.into_iter().collect();

    while let Some(pkg_name) = queue.pop() {
        if let Some(dependents) = reverse_deps.get(pkg_name.as_str()) {
            for &dep in dependents {
                if affected.insert(dep.to_string()) {
                    queue.push(dep.to_string());
                }
            }
        }
    }

    let mut list: Vec<String> = affected.into_iter().collect();
    list.sort();
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_config_change_affects_all_packages() {
        let pkgs = vec![
            WorkspacePkg {
                name: "@app/core".into(),
                dir: PathBuf::from("/root/packages/core"),
                scripts: vec!["build".into()],
                internal_deps: vec![],
            },
            WorkspacePkg {
                name: "@app/ui".into(),
                dir: PathBuf::from("/root/packages/ui"),
                scripts: vec!["build".into()],
                internal_deps: vec!["@app/core".into()],
            },
        ];

        let changed = vec![PathBuf::from("package.json")];
        let affected = resolve_affected_packages(&pkgs, &changed, Path::new("/root"), true);
        assert_eq!(affected.len(), 2);

        let fanout_changed = vec![PathBuf::from("fanout.json")];
        let fanout_affected =
            resolve_affected_packages(&pkgs, &fanout_changed, Path::new("/root"), true);
        assert_eq!(fanout_affected.len(), 2);
    }

    #[test]
    fn direct_and_dependent_cascade() {
        let pkgs = vec![
            WorkspacePkg {
                name: "@app/core".into(),
                dir: PathBuf::from("/root/packages/core"),
                scripts: vec!["build".into()],
                internal_deps: vec![],
            },
            WorkspacePkg {
                name: "@app/ui".into(),
                dir: PathBuf::from("/root/packages/ui"),
                scripts: vec!["build".into()],
                internal_deps: vec!["@app/core".into()],
            },
            WorkspacePkg {
                name: "@app/docs".into(),
                dir: PathBuf::from("/root/apps/docs"),
                scripts: vec!["build".into()],
                internal_deps: vec![],
            },
        ];

        let changed = vec![PathBuf::from("packages/core/src/index.ts")];

        // Without dependents: only @app/core
        let direct_only = resolve_affected_packages(&pkgs, &changed, Path::new("/root"), false);
        assert_eq!(direct_only, vec!["@app/core"]);

        // With dependents: @app/core and @app/ui (docs is unaffected)
        let with_deps = resolve_affected_packages(&pkgs, &changed, Path::new("/root"), true);
        assert_eq!(with_deps, vec!["@app/core", "@app/ui"]);
    }

    #[test]
    fn status_z_keeps_rename_sources_and_spaces() {
        let raw = b" M a/x.ts\0R  b/new.ts\0a/old.ts\0?? c/my file.ts\0";
        let mut out = HashSet::new();
        parse_status_z(raw, &mut out);
        let mut got: Vec<_> = out.into_iter().collect();
        got.sort();
        assert_eq!(
            got,
            ["a/old.ts", "a/x.ts", "b/new.ts", "c/my file.ts"].map(PathBuf::from)
        );
    }

    #[test]
    fn find_changed_files_rejects_flag_refs_and_unknown_refs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let err_flag = find_changed_files(root, "--output=/tmp/pwn").unwrap_err();
        assert_eq!(err_flag, "invalid git ref '--output=/tmp/pwn'");

        let err_unknown = find_changed_files(root, "nonexistent-ref-0000000000000000").unwrap_err();
        assert_eq!(
            err_unknown,
            "unknown or invalid git ref 'nonexistent-ref-0000000000000000'"
        );
    }
}
