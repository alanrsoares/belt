//! Optional per-repo task configuration read from `fanout.json`.
//!
//! Without a config, targets are derived by convention (see `workspace.rs`),
//! which covers repos whose root scripts are named the usual way. A repo whose
//! gate is a specific list — say `check:full` also running a coverage script and
//! two bootstrap north-stars — declares it instead:
//!
//! ```json
//! {
//!   "targets": {
//!     "check":      { "root": ["lint", "typecheck", "fmt:check", "test"], "package": "check" },
//!     "check:full": { "root": ["lint", "typecheck", "test:full", "seed:check"], "package": "check" }
//!   }
//! }
//! ```
//!
//! `root` names scripts in the root `package.json`, run in the order given; a
//! name the root does not declare is skipped, so one config can list gates that
//! only some branches have. `package` names the script run in every workspace
//! package that declares it.
//!
//! `cost` ranks tasks the name heuristic cannot see as long, so they start
//! first instead of setting the wall clock from behind short tasks. Keys are
//! task names as fanout prints them (`seed:check`, `@w/app:test`); values are
//! compared against the built-in ranking, where `test*` scores 3:
//!
//! ```json
//! { "targets": { "check:full": { "root": ["…"], "cost": { "seed:check": 5 } } } }
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::json;

pub const CONFIG_FILE: &str = "fanout.json";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetConfig {
    pub root: Vec<String>,
    pub package: Option<String>,
    pub cost: HashMap<String, usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub targets: HashMap<String, TargetConfig>,
}

impl Config {
    pub fn target(&self, name: &str) -> Option<&TargetConfig> {
        self.targets.get(name)
    }
}

/// Read `fanout.json` from the workspace root. A missing file is not an error —
/// it means "use the conventions". A malformed one is: silently falling back to
/// convention would run a different gate than the repo asked for.
pub fn read(root_dir: &Path) -> Result<Option<Config>, String> {
    let path = root_dir.join(CONFIG_FILE);
    let Ok(content) = fs::read_to_string(&path) else {
        return Ok(None);
    };

    let val =
        json::parse(&content).map_err(|e| format!("failed to parse '{}': {e}", path.display()))?;

    let Some(targets_obj) = val.get("targets").and_then(|t| t.as_object()) else {
        return Err(format!(
            "'{}' has no 'targets' object; expected {{ \"targets\": {{ \"check\": {{ … }} }} }}",
            path.display()
        ));
    };

    let mut targets = HashMap::new();
    for (name, spec) in targets_obj {
        let root = spec
            .get("root")
            .and_then(|r| r.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let package = spec
            .get("package")
            .and_then(|p| p.as_str())
            .map(String::from);

        let cost = match spec.get("cost") {
            None => HashMap::new(),
            Some(c) => read_cost(c).ok_or_else(|| {
                format!(
                    "'{}': target '{name}' has a malformed 'cost'; expected {{ \"<task>\": <non-negative integer> }}",
                    path.display()
                )
            })?,
        };

        targets.insert(
            name.clone(),
            TargetConfig {
                root,
                package,
                cost,
            },
        );
    }

    Ok(Some(Config { targets }))
}

/// `{ "<task>": <integer> }`, or `None` if any value is not a non-negative
/// integer — a typo'd cost should not quietly schedule as zero.
fn read_cost(val: &json::JsonValue) -> Option<HashMap<String, usize>> {
    val.as_object()?
        .iter()
        .map(|(task, v)| match v {
            json::JsonValue::Number(n) if *n >= 0.0 && n.fract() == 0.0 => {
                Some((task.clone(), *n as usize))
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_config_is_not_an_error() {
        let dir = tmp_dir("fanout-test-config-missing");
        assert_eq!(read(&dir).unwrap(), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_root_gates_and_package_script() {
        let dir = tmp_dir("fanout-test-config-read");
        fs::write(
            dir.join(CONFIG_FILE),
            r#"{
                "targets": {
                    "check": { "root": ["lint", "typecheck", "test"], "package": "check" },
                    "check:full": { "root": ["lint", "test:full", "seed:check"] }
                }
            }"#,
        )
        .unwrap();

        let cfg = read(&dir).unwrap().expect("config should parse");
        let check = cfg.target("check").unwrap();
        assert_eq!(check.root, vec!["lint", "typecheck", "test"]);
        assert_eq!(check.package.as_deref(), Some("check"));

        // `package` is optional — the convention default applies.
        let full = cfg.target("check:full").unwrap();
        assert_eq!(full.root, vec!["lint", "test:full", "seed:check"]);
        assert_eq!(full.package, None);

        assert!(cfg.target("lint").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_task_costs() {
        let dir = tmp_dir("fanout-test-config-cost");
        fs::write(
            dir.join(CONFIG_FILE),
            r#"{ "targets": { "check": { "root": ["seed:check"], "cost": { "seed:check": 5 } } } }"#,
        )
        .unwrap();

        let cfg = read(&dir).unwrap().expect("config should parse");
        assert_eq!(
            cfg.target("check").unwrap().cost.get("seed:check"),
            Some(&5)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_cost_is_an_error() {
        let dir = tmp_dir("fanout-test-config-bad-cost");
        for cost in [
            r#""high""#,
            r#"{ "seed:check": "high" }"#,
            r#"{ "seed:check": -1 }"#,
        ] {
            fs::write(
                dir.join(CONFIG_FILE),
                format!(r#"{{ "targets": {{ "check": {{ "cost": {cost} }} }} }}"#),
            )
            .unwrap();
            assert!(read(&dir).is_err(), "cost {cost} should be rejected");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_config_is_an_error() {
        let dir = tmp_dir("fanout-test-config-bad");
        fs::write(dir.join(CONFIG_FILE), "{ not json").unwrap();
        assert!(read(&dir).is_err());

        fs::write(dir.join(CONFIG_FILE), r#"{ "tasks": {} }"#).unwrap();
        assert!(read(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
