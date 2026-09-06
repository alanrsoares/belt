//! Consistent locations for per-tool configuration and data on this machine.
//!
//! Resolution order (first match wins):
//!    1. The matching `XDG_CONFIG_HOME` / `XDG_DATA_HOME` override, if set.
//!    2. A conventional fallback rooted at the user's home directory.
//!
//! On macOS the fallback is XDG-style (`~/.config/...` for config and
//! `~/.local/share/...` for data) rather than `~/Library/...`, matching how
//! the rest of this machine's dotfiles are laid out (`~/.config/zsh`,
//! `~/.config/starship`, `~/.config/mise`, …). On Linux this is already the
//! native convention. Each tool then gets a predictable
//! `~/.config/belt/<tool>/` home.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Parent directory holding one subdirectory per tool:
/// `~/.config/belt/<tool>`.
pub const TOOL_ROOT: &str = "belt";

/// Resolve the user's home directory from `$HOME` (or `%USERPROFILE%` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// XDG config dir for `tool`, e.g. `~/.config/belt/<tool>`.
///
/// Honours `XDG_CONFIG_HOME` when set; otherwise falls back to the home-rooted
/// location. Returns `None` only when `$HOME` is unavailable.
pub fn tool_config_dir(tool: &str) -> Option<PathBuf> {
    if let Some(base) = env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(base).join(TOOL_ROOT).join(tool));
    }
    home_dir().map(|h| join_under(&h, ".config", tool))
}

/// XDG data dir for `tool`, e.g. `~/.local/share/belt/<tool>`.
pub fn tool_data_dir(tool: &str) -> Option<PathBuf> {
    if let Some(base) = env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(base).join(TOOL_ROOT).join(tool));
    }
    home_dir().map(|h| join_under(&h, ".local/share", tool))
}

/// Create `dir` and any missing parents. Idempotent: a pre-existing directory
/// is left untouched and returns `Ok(())`.
pub fn ensure_dir<P: AsRef<Path>>(dir: P) -> std::io::Result<()> {
    fs::create_dir_all(dir.as_ref())
}

/// Pure join helper, kept separate from the env lookups above so the path
/// layout is unit-testable without mutating process-global environment.
pub(crate) fn join_under(root: &Path, subdir: &str, tool: &str) -> PathBuf {
    root.join(subdir).join(TOOL_ROOT).join(tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_under_composes_the_documented_layout() {
        let root = PathBuf::from("/Users/alana");
        assert_eq!(
            join_under(&root, ".config", "scaffold"),
            PathBuf::from("/Users/alana/.config/belt/scaffold")
        );
        assert_eq!(
            join_under(&root, ".local/share", "scaffold"),
            PathBuf::from("/Users/alana/.local/share/belt/scaffold")
        );
    }

    #[test]
    fn ensure_dir_creates_nested_and_is_idempotent() {
        let base = std::env::temp_dir().join("local-common-tests");
        let target = base.join("nested/dir/with");
        let _ = fs::remove_dir_all(&base);

        ensure_dir(&target).expect("first create should succeed");
        assert!(target.is_dir());

        // Second call must be a clean no-op, not an error.
        ensure_dir(&target).expect("idempotent re-create should succeed");
    }
}
