//! Definition of project build artifacts and size formatting.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Known project build artifact categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactType {
    Rust,
    Node,
    Python,
    DotNet,
}

impl ArtifactType {
    /// Parse ecosystem name from CLI flag (e.g. `rust`, `node`, `py`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "rust" | "cargo" | "rs" => Some(ArtifactType::Rust),
            "node" | "ts" | "js" | "bun" | "npm" | "pnpm" | "yarn" => Some(ArtifactType::Node),
            "python" | "py" | "uv" => Some(ArtifactType::Python),
            "dotnet" | "net" | "csharp" => Some(ArtifactType::DotNet),
            _ => None,
        }
    }

    /// Identify artifact type from directory name and optional parent inspection.
    pub fn detect(dir_name: &str, parent: &Path) -> Option<Self> {
        match dir_name {
            // `target` is generic enough to be a user's own directory. Only
            // classify it as Rust output when its project root has a manifest.
            "target" if parent.join("Cargo.toml").is_file() => Some(ArtifactType::Rust),
            "node_modules" | ".next" | ".nuxt" | ".turbo" | ".svelte-kit" | ".astro" | ".vite"
            | ".parcel-cache" | ".nyc_output" => Some(ArtifactType::Node),
            "coverage" if parent.join("package.json").is_file() => Some(ArtifactType::Node),
            ".venv" | "venv"
                if parent.join(dir_name).join("pyvenv.cfg").is_file()
                    || parent.join("pyproject.toml").is_file()
                    || parent.join("requirements.txt").is_file() =>
            {
                Some(ArtifactType::Python)
            }
            "__pycache__" | ".pytest_cache" | ".ruff_cache" | ".mypy_cache" | ".tox" | ".nox" => {
                Some(ArtifactType::Python)
            }
            "bin" | "obj" => {
                if has_dotnet_markers(parent) {
                    Some(ArtifactType::DotNet)
                } else {
                    None
                }
            }
            "dist" | "build" => {
                if parent.join("package.json").is_file() || parent.join("tsconfig.json").is_file() {
                    Some(ArtifactType::Node)
                } else if parent.join("pyproject.toml").is_file()
                    || parent.join("setup.py").is_file()
                {
                    Some(ArtifactType::Python)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ArtifactType::Rust => "Rust",
            ArtifactType::Node => "Node/Bun",
            ArtifactType::Python => "Python",
            ArtifactType::DotNet => ".NET",
        }
    }
}

fn has_dotnet_markers(dir: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Some(ext) = e.path().extension() {
                if ext == "csproj" || ext == "fsproj" || ext == "vbproj" {
                    return true;
                }
            }
        }
    }
    false
}

impl fmt::Display for ArtifactType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// A discovered artifact directory on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub path: PathBuf,
    pub artifact_type: ArtifactType,
    pub size_bytes: u64,
    pub last_modified: Option<SystemTime>,
}

pub use local_common::format_size;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_boundaries() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(2048), "2 KB");
        assert_eq!(format_size(15 * 1024 * 1024), "15.0 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.00 GB");
    }

    #[test]
    fn artifact_type_parsing() {
        assert_eq!(ArtifactType::parse("rust"), Some(ArtifactType::Rust));
        assert_eq!(ArtifactType::parse("ts"), Some(ArtifactType::Node));
        assert_eq!(ArtifactType::parse("pnpm"), Some(ArtifactType::Node));
        assert_eq!(ArtifactType::parse("py"), Some(ArtifactType::Python));
        assert_eq!(ArtifactType::parse("net"), Some(ArtifactType::DotNet));
        assert_eq!(ArtifactType::parse("unknown"), None);
    }

    #[test]
    fn target_requires_a_cargo_manifest() {
        let tmp = std::env::temp_dir().join("devclean-target-detection-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        assert_eq!(ArtifactType::detect("target", &tmp), None);

        std::fs::write(tmp.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        assert_eq!(
            ArtifactType::detect("target", &tmp),
            Some(ArtifactType::Rust)
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn detect_avoids_false_positives_for_coverage_venv_and_dotnet_sln() {
        let tmp = std::env::temp_dir().join("devclean-false-positives-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("venv")).unwrap();

        // coverage without package.json is ignored
        assert_eq!(ArtifactType::detect("coverage", &tmp), None);
        std::fs::write(tmp.join("package.json"), "{}").unwrap();
        assert_eq!(
            ArtifactType::detect("coverage", &tmp),
            Some(ArtifactType::Node)
        );

        // venv without pyvenv.cfg, pyproject.toml, or requirements.txt is ignored
        assert_eq!(ArtifactType::detect("venv", &tmp), None);
        assert_eq!(ArtifactType::detect(".venv", &tmp), None);
        std::fs::write(tmp.join("venv/pyvenv.cfg"), "home = /usr/bin").unwrap();
        assert_eq!(
            ArtifactType::detect("venv", &tmp),
            Some(ArtifactType::Python)
        );

        // bin in a directory with only .sln is ignored; .csproj/.vbproj is detected
        std::fs::write(tmp.join("App.sln"), "").unwrap();
        assert_eq!(ArtifactType::detect("bin", &tmp), None);
        std::fs::write(tmp.join("App.vbproj"), "").unwrap();
        assert_eq!(
            ArtifactType::detect("bin", &tmp),
            Some(ArtifactType::DotNet)
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
