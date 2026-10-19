//! Git-aware change detection for monorepo package scoping (inspired by Turborepo SCM).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

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

    // 1. Commits diff against since_ref
    let diff_target = format!("{since_ref}...HEAD");
    let three_dot = Command::new("git")
        .args(["diff", "--name-only", &diff_target])
        .current_dir(root_dir)
        .output();

    match three_dot {
        Ok(output) if output.status.success() => {
            parse_git_lines(&output.stdout, &mut files);
        }
        _ => {
            // Fallback to direct two-dot diff or single ref if 3-dot fails
            let fallback = Command::new("git")
                .args(["diff", "--name-only", since_ref])
                .current_dir(root_dir)
                .output()
                .map_err(|e| format!("failed to run git diff: {e}"))?;
            if fallback.status.success() {
                parse_git_lines(&fallback.stdout, &mut files);
            } else {
                return Err(format!("unknown or invalid git ref '{since_ref}'"));
            }
        }
    }

    // 2. Unstaged working tree changes
    if let Ok(output) = Command::new("git")
        .args(["diff", "--name-only"])
        .current_dir(root_dir)
        .output()
    {
        if output.status.success() {
            parse_git_lines(&output.stdout, &mut files);
        }
    }

    // 3. Staged changes
    if let Ok(output) = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(root_dir)
        .output()
    {
        if output.status.success() {
            parse_git_lines(&output.stdout, &mut files);
        }
    }

    // 4. Untracked files
    if let Ok(output) = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(root_dir)
        .output()
    {
        if output.status.success() {
            parse_git_lines(&output.stdout, &mut files);
        }
    }

    let mut result: Vec<PathBuf> = files.into_iter().collect();
    result.sort();
    Ok(result)
}

fn parse_git_lines(stdout: &[u8], out: &mut HashSet<PathBuf>) {
    let s = String::from_utf8_lossy(stdout);
    for line in s.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            out.insert(PathBuf::from(trimmed));
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
