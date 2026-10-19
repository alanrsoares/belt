//! High-performance filesystem scanner for project build artifacts.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::artifact::{Artifact, ArtifactType};

/// Options configuring the workspace artifact scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub root: PathBuf,
    pub targets: Vec<ArtifactType>,
    pub min_size_bytes: u64,
    pub older_than: Option<Duration>,
    pub max_depth: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            targets: Vec::new(),
            min_size_bytes: 0,
            older_than: None,
            max_depth: 10,
        }
    }
}

/// Discovered candidate before measuring directory size.
struct Candidate {
    path: PathBuf,
    artifact_type: ArtifactType,
}

/// Scan `root` directory for reclaimable artifacts matching the given options.
pub fn scan(options: &ScanOptions) -> Vec<Artifact> {
    let mut candidates = Vec::new();

    // 1. Fast discovery traversal without computing sizes upfront
    walk_candidates(
        &options.root,
        0,
        options.max_depth,
        options,
        &mut candidates,
    );

    if candidates.is_empty() {
        return Vec::new();
    }

    // 2. Parallel size and mtime measurement across CPU threads
    let pool_size = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(candidates.len())
        .max(1);

    let queue = Arc::new(Mutex::new(VecDeque::from(candidates)));
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();

    for _ in 0..pool_size {
        let q = Arc::clone(&queue);
        let tx_clone = tx.clone();

        let handle = thread::spawn(move || {
            while let Some(candidate) = {
                let mut lock = match q.lock() {
                    Ok(l) => l,
                    Err(p) => p.into_inner(),
                };
                lock.pop_front()
            } {
                let (size_bytes, last_modified) = calculate_dir_size(&candidate.path);
                let _ = tx_clone.send(Artifact {
                    path: candidate.path,
                    artifact_type: candidate.artifact_type,
                    size_bytes,
                    last_modified,
                });
            }
        });
        handles.push(handle);
    }
    drop(tx); // Close initial sender so receiver terminates when threads finish

    let now = SystemTime::now();
    let mut artifacts = Vec::new();

    while let Ok(art) = rx.recv() {
        // Min size filter
        if art.size_bytes < options.min_size_bytes {
            continue;
        }

        // Age filter
        if let Some(req_age) = options.older_than {
            let is_old_enough = art
                .last_modified
                .is_some_and(|time| now.duration_since(time).unwrap_or(Duration::ZERO) >= req_age);
            if !is_old_enough {
                continue;
            }
        }

        artifacts.push(art);
    }

    for h in handles {
        let _ = h.join();
    }

    // Sort by largest size first
    artifacts.sort_by_key(|a| std::cmp::Reverse(a.size_bytes));
    artifacts
}

fn walk_candidates(
    dir: &Path,
    current_depth: usize,
    max_depth: usize,
    opts: &ScanOptions,
    results: &mut Vec<Candidate>,
) {
    if current_depth > max_depth {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };

        // Always skip .git directory
        if file_name == ".git" {
            continue;
        }

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }

        // Check if this directory is a recognized artifact
        if let Some(artifact_type) = ArtifactType::detect(file_name, dir) {
            // Target filter
            if !opts.targets.is_empty() && !opts.targets.contains(&artifact_type) {
                // If not in requested targets, continue walking into subdirectories
                walk_candidates(&path, current_depth + 1, max_depth, opts, results);
                continue;
            }

            results.push(Candidate {
                path,
                artifact_type,
            });

            // Do not descend into detected artifact directories
            continue;
        }

        // Otherwise, continue recursive walk
        walk_candidates(&path, current_depth + 1, max_depth, opts, results);
    }
}

/// Recursively compute total directory size and latest modified timestamp.
pub fn calculate_dir_size(dir: &Path) -> (u64, Option<SystemTime>) {
    let mut total_size: u64 = 0;
    let mut latest_mtime: Option<SystemTime> = fs::symlink_metadata(dir)
        .ok()
        .and_then(|m| m.modified().ok());

    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                if ft.is_symlink() {
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    total_size += meta.len();
                    if let Ok(mtime) = meta.modified() {
                        latest_mtime = match latest_mtime {
                            Some(prev) => Some(prev.max(mtime)),
                            None => Some(mtime),
                        };
                    }
                }
                if ft.is_dir() {
                    stack.push(entry.path());
                }
            }
        }
    }

    (total_size, latest_mtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_mock_artifacts() {
        let tmp = std::env::temp_dir().join("devclean-test-scan");
        let _ = fs::remove_dir_all(&tmp);

        // Create mock rust project
        let rust_proj = tmp.join("rust_proj");
        let rust_target = rust_proj.join("target");
        fs::create_dir_all(&rust_target).unwrap();
        fs::write(rust_proj.join("Cargo.toml"), "[package]").unwrap();
        fs::write(rust_target.join("output.bin"), "1234567890").unwrap();

        // Create mock node project
        let node_proj = tmp.join("node_proj");
        let node_modules = node_proj.join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("pkg.json"), "{}").unwrap();

        let opts = ScanOptions {
            root: tmp.clone(),
            ..Default::default()
        };

        let found = scan(&opts);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|a| a.artifact_type == ArtifactType::Rust));
        assert!(found.iter().any(|a| a.artifact_type == ArtifactType::Node));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn calculate_dir_size_seeds_mtime_and_skips_symlinks() {
        let tmp = std::env::temp_dir().join("devclean-test-dir-size-symlinks");
        let _ = fs::remove_dir_all(&tmp);
        let empty_dir = tmp.join("empty");
        fs::create_dir_all(&empty_dir).unwrap();

        let (empty_size, empty_mtime) = calculate_dir_size(&empty_dir);
        assert_eq!(empty_size, 0);
        assert!(empty_mtime.is_some());

        #[cfg(unix)]
        {
            let loop_dir = tmp.join("loop_dir");
            fs::create_dir_all(&loop_dir).unwrap();
            fs::write(loop_dir.join("data.txt"), "hello").unwrap();
            std::os::unix::fs::symlink(&loop_dir, loop_dir.join("self_link")).unwrap();

            let (size, mtime) = calculate_dir_size(&loop_dir);
            assert!(size >= 5);
            assert!(mtime.is_some());
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scan_older_than_excludes_fresh_and_empty_directories() {
        let tmp = std::env::temp_dir().join("devclean-test-older-than");
        let _ = fs::remove_dir_all(&tmp);

        let node_proj = tmp.join("node_proj");
        let node_modules = node_proj.join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();

        let opts = ScanOptions {
            root: tmp.clone(),
            older_than: Some(Duration::from_secs(3600)),
            ..Default::default()
        };

        let found = scan(&opts);
        assert!(found.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }
}
