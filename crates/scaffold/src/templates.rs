//! Language templates for the [`run`](crate::run) command.
//!
//! Each [`Lang`] renders a complete, ready-to-run project into a plain list of
//! [`File`]s. Rendering is a *pure* function of the target name — no I/O, no
//! shells, no network — which keeps it trivially unit-testable and leaves the
//! binary as a thin I/O wrapper.
//!
//! The generated configs mirror the user's `~/.fns` helpers verbatim
//! (`bun-init-ts`, `rust-init`, `go-init`, `py-init`) so the tool bakes in
//! *their* taste rather than some generic default.

use std::fmt;

/// A single project file: a path relative to the target directory plus its
/// contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Relative path inside the new project, e.g. `"src/main.rs"`.
    pub path: String,
    /// File contents, written verbatim.
    pub contents: String,
}

impl File {
    /// Construct a file.
    pub fn new(path: impl Into<String>, contents: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            contents: contents.into(),
        }
    }
}

/// A language/template the scaffolder can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// Bun + TypeScript + Biome.
    Ts,
    /// Rust binary via `cargo`.
    Rust,
    /// Go module.
    Go,
    /// Python via `uv` / PEP 621.
    Py,
    /// .NET minimal API — not wired in v1 (needs the `dotnet` SDK shell-out).
    DotNet,
}

impl Lang {
    /// Parse a language keyword, accepting the aliases the user's aliases
    /// already use (`ts`, `rust`, `go`, `py`, `net`/`.NET`, plus common
    /// synonyms).
    pub fn parse(s: &str) -> Option<Lang> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ts" | "bun" | "typescript" => Some(Lang::Ts),
            "rust" | "cargo" | "rs" => Some(Lang::Rust),
            "go" | "golang" => Some(Lang::Go),
            "py" | "python" | "uv" => Some(Lang::Py),
            "net" | "dotnet" | ".net" => Some(Lang::DotNet),
            _ => None,
        }
    }

    /// Whether this language is fully rendered by v1.
    pub fn supported(&self) -> bool {
        matches!(self, Lang::Ts | Lang::Rust | Lang::Go | Lang::Py)
    }

    /// The default project name when `--name` is omitted.
    pub fn default_name(&self) -> &'static str {
        match self {
            Lang::Ts => "my-ts-app",
            Lang::Rust => "my-rust-app",
            Lang::Go => "my-go-app",
            Lang::Py => "my-py-app",
            Lang::DotNet => "MyApi",
        }
    }

    /// The short human label used in `--list` output.
    pub fn label(&self) -> &'static str {
        match self {
            Lang::Ts => "Bun + TypeScript + Biome",
            Lang::Rust => "Rust binary (cargo)",
            Lang::Go => "Go module",
            Lang::Py => "Python (uv / PEP 621)",
            Lang::DotNet => ".NET minimal API (v2)",
        }
    }

    /// Render every file for a project called `name`. Pure: no I/O.
    pub fn render(&self, name: &str) -> Vec<File> {
        let name = sanitize_name(name);
        match self {
            Lang::Ts => ts(&name),
            Lang::Rust => rust(&name),
            Lang::Go => go(&name),
            Lang::Py => py(&name),
            // Unsupported in v1: return empty so the caller can branch on
            // `supported()` and print guidance instead of a hollow tree.
            Lang::DotNet => Vec::new(),
        }
    }

    /// Every language, in help/list/display order.
    pub fn all() -> [Lang; 5] {
        [Lang::Ts, Lang::Rust, Lang::Go, Lang::Py, Lang::DotNet]
    }
}

impl Lang {
    /// The follow-up command to hint the user runs after scaffolding, using the
    /// short aliases from ~/.aliases.
    pub fn next_hint(&self) -> &'static str {
        match self {
            Lang::Ts => "bun install && bd",
            Lang::Rust => "cgr",
            Lang::Go => "gor",
            Lang::Py => "uv run main.py",
            Lang::DotNet => "dnw",
        }
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Lang::Ts => "ts",
            Lang::Rust => "rust",
            Lang::Go => "go",
            Lang::Py => "py",
            Lang::DotNet => "dotnet",
        };
        f.pad(s)
    }
}

/// Collapse a project name to a valid package-identifier-ish string.
///
/// Rejects path separators so `--name ../evil` can never escape the target
/// directory; spaces and other non-identifier chars become hyphens and
/// consecutive hyphens are collapsed. The always-valid `app` is used when the
/// sanitized result is empty.
pub(crate) fn sanitize_name(name: &str) -> String {
    if name.trim().is_empty() || name.contains('/') || name.contains('\\') {
        return "app".to_string();
    }
    let mut replaced = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            replaced.push(ch);
        } else {
            replaced.push('-');
        }
    }
    let collapsed: String = replaced
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let out = collapsed.trim_end_matches('.').to_string();
    if out.is_empty() {
        "app".to_string()
    } else {
        out
    }
}

// --- per-language renderers ------------------------------------------------
//
// The brace-heavy `package.json` uses a `__NAME__` placeholder so we avoid
// `format!` brace-escaping; the other templates are single-level and safe to
// interpolate with `format!`.

fn ts(name: &str) -> Vec<File> {
    let pkg = r#"{
  "name": "__NAME__",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "bun --watch run index.ts",
    "start": "bun run index.ts",
    "build": "bun build ./index.ts --outfile ./dist/index.js",
    "test": "bun test",
    "typecheck": "tsc --noEmit",
    "fmt": "biome format --write .",
    "lint": "biome check . --error-on-warnings",
    "check": "fanout check"
  },
  "devDependencies": {
    "@biomejs/biome": "2.0.0",
    "bun-types": "latest",
    "typescript": "latest"
  }
}"#
    .replace("__NAME__", name);

    vec![
        File::new("package.json", pkg),
        // The user's exact tsconfig from ~/.fns `bun-init-ts`.
        File::new(
            "tsconfig.json",
            r#"{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "types": ["bun-types"],
    "strict": true,
    "skipLibCheck": true,
    "noEmit": true,
    "isolatedModules": true
  }
}"#,
        ),
        File::new(
            "biome.json",
            r#"{
  "$schema": "https://biomejs.dev/schemas/2.0.0/schema.json",
  "formatter": { "enabled": true, "indentStyle": "space", "indentWidth": 4 },
  "linter": { "enabled": true },
  "assist": { "actions": { "source": { "organizeImports": "on" } } }
}"#,
        ),
        File::new(
            "index.ts",
            r#"// ⚡ Scaffolded by `scaffold ts`.
console.log("⚡ Hello from Bun + TypeScript on macOS M5!");
"#,
        ),
        File::new(".gitignore", "node_modules/\ndist/\n*.log\n.DS_Store\n"),
    ]
}

fn rust(name: &str) -> Vec<File> {
    vec![
        File::new(
            "Cargo.toml",
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ),
        File::new(
            "src/main.rs",
            "fn main() {\n    println!(\"⚡ Hello from Rust on macOS M5!\");\n}\n",
        ),
        File::new(".gitignore", "/target/\n*.log\n.DS_Store\n"),
    ]
}

fn go(name: &str) -> Vec<File> {
    vec![
        File::new("go.mod", format!("module {name}\n\ngo 1.23\n")),
        // The user's exact main.go — note the tab indentation (gofmt-clean).
        File::new(
            "main.go",
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"⚡ Hello from Go on macOS M5!\")\n}\n",
        ),
        File::new(".gitignore", "*.out\n.DS_Store\n"),
    ]
}

fn py(name: &str) -> Vec<File> {
    vec![
        File::new(
            "pyproject.toml",
            format!(
                "[project]\nname = \"{name}\"\nversion = \"0.1.0\"\nrequires-python = \">=3.13\"\ndescription = \"Scaffolded by `scaffold py`.\"\nreadme = \"README.md\"\ndependencies = []\n\n[project.scripts]\n\n[tool.uv]\n"
            ),
        ),
        File::new(
            "main.py",
            "def main() -> None:\n    print(\"⚡ Hello from Python on macOS M5!\")\n\n\nif __name__ == \"__main__\":\n    main()\n",
        ),
        File::new(
            "README.md",
            format!("# {name}\n\nScaffolded with `scaffold py` (uv / PEP 621).\n\nRun: `uv run main.py`\n"),
        ),
        File::new(".gitignore", "__pycache__/\n.venv/\n*.pyc\n.DS_Store\n"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_parsing_aliases() {
        assert_eq!(Lang::parse("ts"), Some(Lang::Ts));
        assert_eq!(Lang::parse("bun"), Some(Lang::Ts));
        assert_eq!(Lang::parse("typescript"), Some(Lang::Ts));

        assert_eq!(Lang::parse("rust"), Some(Lang::Rust));
        assert_eq!(Lang::parse("cargo"), Some(Lang::Rust));
        assert_eq!(Lang::parse("rs"), Some(Lang::Rust));

        assert_eq!(Lang::parse("go"), Some(Lang::Go));
        assert_eq!(Lang::parse("golang"), Some(Lang::Go));

        assert_eq!(Lang::parse("py"), Some(Lang::Py));
        assert_eq!(Lang::parse("python"), Some(Lang::Py));
        assert_eq!(Lang::parse("uv"), Some(Lang::Py));

        assert_eq!(Lang::parse("net"), Some(Lang::DotNet));
        assert_eq!(Lang::parse("dotnet"), Some(Lang::DotNet));
        assert_eq!(Lang::parse(".net"), Some(Lang::DotNet));

        assert_eq!(Lang::parse("invalid"), None);
    }

    #[test]
    fn sanitize_name_edge_cases() {
        assert_eq!(sanitize_name("my-app"), "my-app");
        assert_eq!(sanitize_name("my app"), "my-app");
        assert_eq!(sanitize_name("my---app"), "my-app");
        assert_eq!(sanitize_name("../evil"), "app");
        assert_eq!(sanitize_name(r"..\evil"), "app");
        assert_eq!(sanitize_name(""), "app");
        assert_eq!(sanitize_name("   "), "app");
        assert_eq!(sanitize_name("---"), "app");
        assert_eq!(sanitize_name("foo.bar"), "foo.bar");
    }

    #[test]
    fn lang_render_outputs() {
        let ts_files = Lang::Ts.render("cool-ts");
        assert!(ts_files.iter().any(|f| f.path == "package.json"));
        assert!(ts_files.iter().any(|f| f.path == "index.ts"));

        let rust_files = Lang::Rust.render("cool-rs");
        assert!(rust_files.iter().any(|f| f.path == "Cargo.toml"));
        assert!(rust_files.iter().any(|f| f.path == "src/main.rs"));

        let go_files = Lang::Go.render("cool-go");
        assert!(go_files.iter().any(|f| f.path == "go.mod"));
        assert!(go_files.iter().any(|f| f.path == "main.go"));

        let py_files = Lang::Py.render("cool-py");
        assert!(py_files.iter().any(|f| f.path == "pyproject.toml"));
        assert!(py_files.iter().any(|f| f.path == "main.py"));

        let net_files = Lang::DotNet.render("cool-net");
        assert!(net_files.is_empty());
    }
}
