//! Topological task dependency graph and scheduling (inspired by Turborepo Engine).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::workspace::{TaskSpec, WorkspacePkg};

#[derive(Debug, Clone, Default)]
pub struct TurboPipeline {
    pub task_rules: HashMap<String, PipelineRule>,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineRule {
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TaskNode {
    pub id: usize,
    pub spec: TaskSpec,
    pub depends_on: HashSet<usize>,
    pub dependents: HashSet<usize>,
}

#[derive(Debug, Clone)]
pub struct TaskGraph {
    pub nodes: Vec<TaskNode>,
}

impl TaskGraph {
    /// Construct a DAG from a flat task list, package manifests, and optional turbo pipeline rules.
    pub fn build(
        tasks: Vec<TaskSpec>,
        pkgs: &[WorkspacePkg],
        pipeline: Option<&TurboPipeline>,
        topological: bool,
    ) -> Result<Self, String> {
        let mut pkg_map: HashMap<&str, &WorkspacePkg> = HashMap::new();
        for pkg in pkgs {
            pkg_map.insert(&pkg.name, pkg);
        }

        let mut nodes: Vec<TaskNode> = tasks
            .into_iter()
            .enumerate()
            .map(|(id, spec)| TaskNode {
                id,
                spec,
                depends_on: HashSet::new(),
                dependents: HashSet::new(),
            })
            .collect();

        // Map "pkg_name:task_name" or root task name -> task index
        let mut task_lookup: HashMap<String, usize> = HashMap::new();
        for (i, node) in nodes.iter().enumerate() {
            task_lookup.insert(node.spec.name.clone(), i);
        }

        // Resolve dependencies
        for i in 0..nodes.len() {
            let pkg_name = nodes[i].spec.pkg_name.clone();
            let script_name = nodes[i].spec.script_name.clone();

            let mut depends_on_rules = if let Some(rules) =
                pipeline.and_then(|p| p.task_rules.get(script_name.as_str()))
            {
                rules.depends_on.clone()
            } else if script_name == "build" || topological {
                // Default heuristic: build tasks (or any task under --topological) depend on upstream packages
                vec![format!("^{script_name}")]
            } else {
                Vec::new()
            };

            if topological {
                let topo_rule = format!("^{script_name}");
                if !depends_on_rules.contains(&topo_rule) {
                    depends_on_rules.push(topo_rule);
                }
            }

            for rule in depends_on_rules {
                if let Some(dep_script) = rule.strip_prefix('^') {
                    // Package dependencies' task (e.g. ^build)
                    if let Some(pkg) = pkg_name.as_deref().and_then(|p| pkg_map.get(p)) {
                        for internal_dep in &pkg.internal_deps {
                            let dep_task_name = format!("{internal_dep}:{dep_script}");
                            if let Some(&dep_idx) = task_lookup.get(&dep_task_name) {
                                nodes[i].depends_on.insert(dep_idx);
                                nodes[dep_idx].dependents.insert(i);
                            }
                        }
                    }
                } else if let Some(ref pkg) = pkg_name {
                    // Same-package task dependency (e.g. "build" before "test")
                    let dep_task_name = format!("{pkg}:{rule}");
                    if let Some(&dep_idx) = task_lookup.get(&dep_task_name) {
                        nodes[i].depends_on.insert(dep_idx);
                        nodes[dep_idx].dependents.insert(i);
                    }
                } else if let Some(&dep_idx) = task_lookup.get(&rule) {
                    // Root task dependency
                    nodes[i].depends_on.insert(dep_idx);
                    nodes[dep_idx].dependents.insert(i);
                }
            }
        }

        let graph = Self { nodes };
        graph.detect_cycles()?;
        Ok(graph)
    }

    /// Kahn's algorithm cycle detection and topological sorting.
    pub fn detect_cycles(&self) -> Result<(), String> {
        let mut in_degrees: Vec<usize> = self.nodes.iter().map(|n| n.depends_on.len()).collect();
        let mut queue: VecDeque<usize> = VecDeque::new();

        for (i, &deg) in in_degrees.iter().enumerate() {
            if deg == 0 {
                queue.push_back(i);
            }
        }

        let mut visited_count = 0;
        while let Some(node_idx) = queue.pop_front() {
            visited_count += 1;
            for &dep_idx in &self.nodes[node_idx].dependents {
                in_degrees[dep_idx] -= 1;
                if in_degrees[dep_idx] == 0 {
                    queue.push_back(dep_idx);
                }
            }
        }

        if visited_count != self.nodes.len() {
            return Err("cyclical task dependency detected in workspace".to_string());
        }

        Ok(())
    }

    /// Return tasks with in-degree 0 that can start immediately.
    pub fn ready_tasks(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.depends_on.is_empty())
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn topological_dependency_resolution() {
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

        let tasks = vec![
            TaskSpec {
                name: "@app/ui:build".into(),
                pkg_name: Some("@app/ui".into()),
                script_name: "build".into(),
                runner_bin: "bun".into(),
                args: vec!["run".into(), "build".into()],
                cwd: PathBuf::from("/root/packages/ui"),
                color_idx: 0,
                estimated_cost: 1,
            },
            TaskSpec {
                name: "@app/core:build".into(),
                pkg_name: Some("@app/core".into()),
                script_name: "build".into(),
                runner_bin: "bun".into(),
                args: vec!["run".into(), "build".into()],
                cwd: PathBuf::from("/root/packages/core"),
                color_idx: 1,
                estimated_cost: 1,
            },
        ];

        let graph = TaskGraph::build(tasks, &pkgs, None, false).expect("DAG build failed");

        // @app/core:build has index 1, @app/ui:build has index 0
        // @app/ui:build should depend on index 1
        assert!(graph.nodes[0].depends_on.contains(&1));
        assert!(graph.nodes[1].dependents.contains(&0));

        // Ready tasks should only be @app/core:build (index 1)
        assert_eq!(graph.ready_tasks(), vec![1]);
    }

    #[test]
    fn root_task_with_colon_is_not_misparsed_as_package_task() {
        let pkgs = vec![WorkspacePkg {
            name: "fmt".into(),
            dir: PathBuf::from("/root/packages/fmt"),
            scripts: vec!["check".into()],
            internal_deps: vec![],
        }];

        let mut rules = HashMap::new();
        rules.insert(
            "fmt:check".to_string(),
            PipelineRule {
                depends_on: vec!["lint".to_string()],
            },
        );
        let pipeline = TurboPipeline { task_rules: rules };

        let tasks = vec![
            TaskSpec {
                name: "lint".into(),
                pkg_name: None,
                script_name: "lint".into(),
                runner_bin: "bun".into(),
                args: vec!["run".into(), "lint".into()],
                cwd: PathBuf::from("/root"),
                color_idx: 0,
                estimated_cost: 0,
            },
            TaskSpec {
                name: "fmt:check".into(),
                pkg_name: None,
                script_name: "fmt:check".into(),
                runner_bin: "bun".into(),
                args: vec!["run".into(), "fmt:check".into()],
                cwd: PathBuf::from("/root"),
                color_idx: 1,
                estimated_cost: 0,
            },
        ];

        let graph =
            TaskGraph::build(tasks, &pkgs, Some(&pipeline), false).expect("DAG build failed");
        assert!(graph.nodes[1].depends_on.contains(&0));
        assert_eq!(graph.ready_tasks(), vec![0]);
    }

    #[test]
    fn topological_flag_orders_non_build_scripts_without_turbo_json() {
        let pkgs = vec![
            WorkspacePkg {
                name: "@app/core".into(),
                dir: PathBuf::from("/root/packages/core"),
                scripts: vec!["typecheck".into()],
                internal_deps: vec![],
            },
            WorkspacePkg {
                name: "@app/ui".into(),
                dir: PathBuf::from("/root/packages/ui"),
                scripts: vec!["typecheck".into()],
                internal_deps: vec!["@app/core".into()],
            },
        ];

        let tasks = vec![
            TaskSpec {
                name: "@app/ui:typecheck".into(),
                pkg_name: Some("@app/ui".into()),
                script_name: "typecheck".into(),
                runner_bin: "bun".into(),
                args: vec!["run".into(), "typecheck".into()],
                cwd: PathBuf::from("/root/packages/ui"),
                color_idx: 0,
                estimated_cost: 1,
            },
            TaskSpec {
                name: "@app/core:typecheck".into(),
                pkg_name: Some("@app/core".into()),
                script_name: "typecheck".into(),
                runner_bin: "bun".into(),
                args: vec!["run".into(), "typecheck".into()],
                cwd: PathBuf::from("/root/packages/core"),
                color_idx: 1,
                estimated_cost: 1,
            },
        ];

        // Without --topological, typecheck tasks run in parallel
        let flat = TaskGraph::build(tasks.clone(), &pkgs, None, false).unwrap();
        assert_eq!(flat.ready_tasks(), vec![0, 1]);

        // With --topological, @app/ui:typecheck waits for @app/core:typecheck
        let topo = TaskGraph::build(tasks, &pkgs, None, true).unwrap();
        assert!(topo.nodes[0].depends_on.contains(&1));
        assert_eq!(topo.ready_tasks(), vec![1]);
    }
}
