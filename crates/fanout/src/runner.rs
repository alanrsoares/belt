//! Concurrent execution engine, topological DAG scheduler, and UI event loop for `fanout`.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use local_common::is_terminal;

use crate::cli::Options;
use crate::dag::{TaskGraph, TurboPipeline};
use crate::ui::{
    draw_meter, format_duration, phase_icon, phase_text, terminal_columns, visible_width,
    CursorGuard, Outcome, Styles,
};
use crate::workspace::{TaskSpec, WorkspacePkg};

pub enum Event {
    TaskStarted {
        index: usize,
    },
    TaskLine {
        index: usize,
        text: String,
        is_stderr: bool,
    },
    TaskFinished {
        index: usize,
        outcome: Outcome,
        duration: Duration,
        exit_code: i32,
        tail_lines: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct TaskState {
    pub phase: Outcome,
    pub began: Option<Instant>,
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct TaskResult {
    pub name: String,
    pub outcome: Outcome,
    pub duration: Duration,
    pub color_idx: usize,
}

struct SchedulerState {
    queue: VecDeque<usize>,
    in_degrees: Vec<usize>,
    completed: usize,
    bail: bool,
}

pub fn execute_tasks(
    tasks: Vec<TaskSpec>,
    pkgs: Vec<WorkspacePkg>,
    pipeline: Option<TurboPipeline>,
    opts: Options,
) -> i32 {
    let num_tasks = tasks.len();
    if num_tasks == 0 {
        println!("no tasks to run");
        return 0;
    }

    let stdout_is_tty = is_terminal(&io::stdout());
    let color_enabled = if opts.color {
        true
    } else if opts.no_color || opts.compact {
        false
    } else {
        stdout_is_tty && std::env::var_os("NO_COLOR").is_none()
    };

    let styles = Styles::new(color_enabled);
    let interactive = stdout_is_tty && !opts.compact;
    let _cursor_guard = CursorGuard::new(interactive);

    let gutter = tasks.iter().map(|t| t.name.len()).max().unwrap_or(10);
    let start_time = Instant::now();

    // Print plan header if not in compact mode
    if !opts.compact {
        print_plan_header(&tasks, &opts, &styles);
    }

    // Build task dependency graph
    let graph = match TaskGraph::build(tasks.clone(), &pkgs, pipeline.as_ref()) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("fanout: {e}");
            return 1;
        }
    };

    let (tx, rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();
    let bail_flag = Arc::new(AtomicBool::new(false));
    let active_workers = Arc::new(AtomicUsize::new(0));

    // Initialize scheduler with in-degrees and ready tasks (in-degree == 0)
    let in_degrees: Vec<usize> = graph.nodes.iter().map(|n| n.depends_on.len()).collect();
    let mut ready_tasks: Vec<usize> = graph.ready_tasks();

    // Sort ready tasks by cost (longest poles first)
    ready_tasks.sort_by(|&a, &b| tasks[b].estimated_cost.cmp(&tasks[a].estimated_cost));

    let scheduler = Arc::new((
        Mutex::new(SchedulerState {
            queue: VecDeque::from(ready_tasks),
            in_degrees,
            completed: 0,
            bail: false,
        }),
        Condvar::new(),
    ));

    let pool_size = opts.jobs.min(num_tasks);
    let mut worker_handles = Vec::new();

    for _ in 0..pool_size {
        let sched = Arc::clone(&scheduler);
        let tasks_ref = tasks.clone();
        let graph_ref = graph.clone();
        let tx_clone = tx.clone();
        let bail_ref = Arc::clone(&bail_flag);
        let active = Arc::clone(&active_workers);
        let timeout_ms = opts.timeout_ms;
        let tail_cap = opts.tail_lines;
        let bail_on_fail = opts.bail;
        let color_opt = color_enabled;

        let handle = thread::spawn(move || loop {
            let (lock, cvar) = &*sched;

            let next_idx = {
                let mut state = match lock.lock() {
                    Ok(l) => l,
                    Err(p) => p.into_inner(),
                };

                loop {
                    if state.bail || state.completed >= num_tasks || bail_ref.load(Ordering::SeqCst)
                    {
                        return;
                    }

                    if let Some(idx) = state.queue.pop_front() {
                        break idx;
                    }

                    if active.load(Ordering::SeqCst) == 0 && state.queue.is_empty() {
                        // Deadlock prevention / unreachable remaining tasks
                        return;
                    }

                    let (new_state, _) = match cvar.wait_timeout(state, Duration::from_millis(50)) {
                        Ok(pair) => pair,
                        Err(p) => p.into_inner(),
                    };
                    state = new_state;
                }
            };

            active.fetch_add(1, Ordering::SeqCst);
            let _ = tx_clone.send(Event::TaskStarted { index: next_idx });

            let spec = &tasks_ref[next_idx];
            let task_start = Instant::now();

            let (outcome, exit_code, tail_lines) = run_single_task(
                spec, timeout_ms, tail_cap, &tx_clone, next_idx, &bail_ref, color_opt,
            );

            let duration = task_start.elapsed();
            active.fetch_sub(1, Ordering::SeqCst);

            // Update DAG and unlock dependents
            {
                let mut state = match lock.lock() {
                    Ok(l) => l,
                    Err(p) => p.into_inner(),
                };
                state.completed += 1;

                if outcome.is_bad() && bail_on_fail {
                    state.bail = true;
                    bail_ref.store(true, Ordering::SeqCst);
                } else if outcome == Outcome::Passed {
                    // Unlock dependents whose prerequisites have all passed
                    for &dep_idx in &graph_ref.nodes[next_idx].dependents {
                        if state.in_degrees[dep_idx] > 0 {
                            state.in_degrees[dep_idx] -= 1;
                            if state.in_degrees[dep_idx] == 0 {
                                state.queue.push_back(dep_idx);
                            }
                        }
                    }
                }

                cvar.notify_all();
            }

            let _ = tx_clone.send(Event::TaskFinished {
                index: next_idx,
                outcome,
                duration,
                exit_code,
                tail_lines,
            });
        });

        worker_handles.push(handle);
    }

    drop(tx); // Drop initial sender so rx loop completes when workers finish

    // TUI event loop state
    let mut states: Vec<TaskState> = vec![
        TaskState {
            phase: Outcome::Pending,
            began: None,
            duration: None,
        };
        num_tasks
    ];
    let mut buffered_logs: Vec<Vec<String>> = vec![Vec::new(); num_tasks];
    let mut results: Vec<Option<TaskResult>> = vec![None; num_tasks];
    let mut painted_lines = 0;
    let mut frame: usize = 0;
    let mut last_tick = Instant::now();

    let mut out = io::BufWriter::with_capacity(64 * 1024, io::stdout());

    loop {
        let mut state_changed = false;
        let mut has_output = false;

        while let Ok(event) = rx.try_recv() {
            match event {
                Event::TaskStarted { index } => {
                    states[index].phase = Outcome::Running;
                    states[index].began = Some(Instant::now());
                    state_changed = true;
                }
                Event::TaskLine {
                    index,
                    text,
                    is_stderr,
                } => {
                    if interactive && painted_lines > 0 {
                        clear_pinned(&mut out, painted_lines);
                        painted_lines = 0;
                    }
                    has_output = true;

                    let spec = &tasks[index];
                    let prefix = format!(
                        "{}{}{}{} {}{}",
                        styles.task_color(spec.color_idx),
                        styles.bold(),
                        spec.name.pad_right(gutter),
                        styles.reset(),
                        styles.gray(),
                        if is_stderr { "┃" } else { "│" }
                    );
                    let rendered = format!("{}{}{}\n", prefix, styles.reset(), text);
                    if interactive {
                        let _ = out.write_all(rendered.as_bytes());
                    } else if !opts.compact {
                        buffered_logs[index].push(rendered);
                    }
                }
                Event::TaskFinished {
                    index,
                    outcome,
                    duration,
                    exit_code,
                    tail_lines,
                } => {
                    if interactive && painted_lines > 0 {
                        clear_pinned(&mut out, painted_lines);
                        painted_lines = 0;
                    }
                    has_output = true;

                    states[index].phase = outcome;
                    states[index].duration = Some(duration);
                    state_changed = true;
                    let spec = &tasks[index];

                    results[index] = Some(TaskResult {
                        name: spec.name.clone(),
                        outcome,
                        duration,
                        color_idx: spec.color_idx,
                    });

                    if interactive {
                        let verdict =
                            format_verdict(spec, outcome, duration, exit_code, gutter, &styles);
                        let _ = out.write_all(verdict.as_bytes());
                    } else if opts.compact {
                        if outcome != Outcome::Passed && outcome != Outcome::Cancelled {
                            let _ = writeln!(
                                out,
                                "{} {} ({})",
                                match outcome {
                                    Outcome::Failed(_) => "FAILED",
                                    Outcome::Timeout => "TIMEOUT",
                                    _ => "ERROR",
                                },
                                spec.name,
                                format_duration(duration)
                            );
                            if !tail_lines.is_empty() {
                                let _ = writeln!(out, "--- {}", spec.name);
                                for line in &tail_lines {
                                    let _ = writeln!(out, "{line}");
                                }
                            }
                        }
                    } else {
                        // Non-interactive standard output
                        for line in &buffered_logs[index] {
                            let _ = out.write_all(line.as_bytes());
                        }
                        let verdict =
                            format_verdict(spec, outcome, duration, exit_code, gutter, &styles);
                        let _ = out.write_all(verdict.as_bytes());
                    }
                }
            }
        }

        if last_tick.elapsed() >= Duration::from_millis(80) {
            frame = frame.wrapping_add(1);
            last_tick = Instant::now();
            state_changed = true;
        }

        if interactive && (state_changed || has_output || painted_lines == 0) {
            clear_pinned(&mut out, painted_lines);
            painted_lines = paint_status_block(
                &mut out, &tasks, &states, frame, start_time, gutter, &styles,
            );
            let _ = out.flush();
        }

        // Check completion
        let settled_count = states.iter().filter(|st| st.phase.is_settled()).count();
        if settled_count == num_tasks {
            break;
        }

        // Check if all workers exited (e.g. on bail or deadlock)
        if active_workers.load(Ordering::SeqCst) == 0 {
            let sched_lock = scheduler.0.lock().unwrap();
            if sched_lock.queue.is_empty() {
                break;
            }
        }

        thread::sleep(Duration::from_millis(15));
    }

    if interactive {
        clear_pinned(&mut out, painted_lines);
        let _ = out.flush();
    }

    // Join workers
    for h in worker_handles {
        let _ = h.join();
    }

    // Mark remaining pending tasks as cancelled if aborted early
    for i in 0..num_tasks {
        if results[i].is_none() {
            let spec = &tasks[i];
            results[i] = Some(TaskResult {
                name: spec.name.clone(),
                outcome: Outcome::Cancelled,
                duration: Duration::ZERO,
                color_idx: spec.color_idx,
            });
        }
    }

    let final_results: Vec<TaskResult> = results.into_iter().flatten().collect();
    let total_elapsed = start_time.elapsed();

    if !opts.compact {
        print_verdict_summary(&final_results, total_elapsed, gutter, &styles);
    }

    let has_failures = final_results.iter().any(|r| r.outcome.is_bad());
    if has_failures {
        1
    } else {
        0
    }
}

fn print_plan_header(tasks: &[TaskSpec], opts: &Options, s: &Styles) {
    let target_label = opts.targets.join(", ");
    println!(
        "{}⚡ fanout v{}{} running {}{}{} for {} task{} (concurrency: {}{}{})\n",
        s.bold(),
        crate::cli::VERSION,
        s.reset(),
        s.cyan(),
        target_label,
        s.reset(),
        tasks.len(),
        if tasks.len() == 1 { "" } else { "s" },
        s.bold(),
        opts.jobs.min(tasks.len()),
        s.reset()
    );
}

pub(crate) fn resolve_runner_bin(bin: &str) -> String {
    #[cfg(windows)]
    {
        if bin.ends_with(".exe") || bin.ends_with(".cmd") || bin.ends_with(".bat") {
            return bin.to_string();
        }
        if matches!(bin, "npm" | "pnpm" | "yarn") {
            if let Ok(path_var) = std::env::var("PATH") {
                for dir in std::env::split_paths(&path_var) {
                    let cmd_path = dir.join(format!("{bin}.cmd"));
                    if cmd_path.is_file() {
                        return cmd_path.to_string_lossy().to_string();
                    }
                }
            }
            return format!("{bin}.cmd");
        }
        bin.to_string()
    }
    #[cfg(not(windows))]
    {
        bin.to_string()
    }
}

fn run_single_task(
    spec: &TaskSpec,
    timeout_ms: u64,
    tail_cap: usize,
    tx: &Sender<Event>,
    idx: usize,
    bail_ref: &AtomicBool,
    color_opt: bool,
) -> (Outcome, i32, Vec<String>) {
    let runner = resolve_runner_bin(&spec.runner_bin);
    let mut cmd = Command::new(&runner);
    cmd.args(&spec.args)
        .current_dir(&spec.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    local_common::process::own_process_group(&mut cmd);

    if color_opt {
        cmd.env("FORCE_COLOR", "1");
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Event::TaskLine {
                index: idx,
                text: format!("spawn failed: {e}"),
                is_stderr: true,
            });
            return (Outcome::Failed(1), 1, vec![format!("spawn error: {e}")]);
        }
    };

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let collected_tail = Arc::new(Mutex::new(VecDeque::new()));

    let t1_tail = Arc::clone(&collected_tail);
    let t1_tx = tx.clone();
    let h_stdout = thread::spawn(move || {
        if let Some(pipe) = stdout_pipe {
            let reader = BufReader::new(pipe);
            for line in reader.lines().map_while(Result::ok) {
                {
                    let mut t = match t1_tail.lock() {
                        Ok(l) => l,
                        Err(p) => p.into_inner(),
                    };
                    t.push_back(line.clone());
                    if tail_cap > 0 && t.len() > tail_cap {
                        t.pop_front();
                    }
                }
                let _ = t1_tx.send(Event::TaskLine {
                    index: idx,
                    text: line,
                    is_stderr: false,
                });
            }
        }
    });

    let t2_tail = Arc::clone(&collected_tail);
    let t2_tx = tx.clone();
    let h_stderr = thread::spawn(move || {
        if let Some(pipe) = stderr_pipe {
            let reader = BufReader::new(pipe);
            for line in reader.lines().map_while(Result::ok) {
                {
                    let mut t = match t2_tail.lock() {
                        Ok(l) => l,
                        Err(p) => p.into_inner(),
                    };
                    t.push_back(line.clone());
                    if tail_cap > 0 && t.len() > tail_cap {
                        t.pop_front();
                    }
                }
                let _ = t2_tx.send(Event::TaskLine {
                    index: idx,
                    text: line,
                    is_stderr: true,
                });
            }
        }
    });

    let task_start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);

    let (outcome, exit_code) = loop {
        if bail_ref.load(Ordering::SeqCst) {
            local_common::process::terminate(&mut child);
            break (Outcome::Cancelled, 0);
        }

        if task_start.elapsed() >= timeout {
            local_common::process::terminate(&mut child);
            break (Outcome::Timeout, 124);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    break (Outcome::Passed, 0);
                } else {
                    let code = status.code().unwrap_or(1);
                    break (Outcome::Failed(code), code);
                }
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                break (Outcome::Failed(1), 1);
            }
        }
    };

    let _ = h_stdout.join();
    let _ = h_stderr.join();

    let tail_lines: Vec<String> = {
        let lock = match collected_tail.lock() {
            Ok(l) => l,
            Err(p) => p.into_inner(),
        };
        lock.iter().cloned().collect()
    };

    (outcome, exit_code, tail_lines)
}

fn clear_pinned(out: &mut impl Write, count: usize) {
    if count > 0 {
        let _ = write!(out, "\x1b[{count}F\x1b[0J");
    }
}

fn paint_status_block(
    out: &mut impl Write,
    tasks: &[TaskSpec],
    states: &[TaskState],
    frame: usize,
    start_time: Instant,
    gutter: usize,
    s: &Styles,
) -> usize {
    let mut lines = Vec::new();
    lines.push(String::new()); // blank line separator

    // Header line
    let settled = states.iter().filter(|st| st.phase.is_settled()).count();
    let failed = states.iter().filter(|st| st.phase.is_bad()).count();
    let ratio = if tasks.is_empty() {
        0.0
    } else {
        settled as f64 / tasks.len() as f64
    };
    let width = 24
        .min(terminal_columns().saturating_sub(gutter + 34))
        .max(10);
    let bar_color = if failed > 0 { s.red() } else { s.cyan() };
    let meter = draw_meter(ratio, width, bar_color, s);

    let header = format!(
        "{} {}/{} {} [{}] {}{}",
        s.bold(),
        settled,
        tasks.len(),
        meter,
        format_duration(start_time.elapsed()),
        if failed > 0 {
            format!("{}{} failed{}", s.red(), failed, s.reset())
        } else {
            String::new()
        },
        s.reset()
    );
    lines.push(header);

    // One line per task
    for (i, spec) in tasks.iter().enumerate() {
        let st = &states[i];
        let icon = phase_icon(st.phase, s, frame);
        let color = s.task_color(spec.color_idx);
        let name_padded = spec.name.pad_right(gutter);

        let dur_str = match (st.phase, st.began, st.duration) {
            (Outcome::Running, Some(b), _) => format_duration(b.elapsed()),
            (_, _, Some(d)) => format_duration(d),
            _ => String::new(),
        };

        let phase_str = phase_text(st.phase, s);

        let line = format!(
            "  {} {}{}{} {} {}{}{}",
            icon,
            color,
            name_padded,
            s.reset(),
            phase_str,
            s.gray(),
            if dur_str.is_empty() {
                String::new()
            } else {
                format!("({})", dur_str)
            },
            s.reset()
        );
        lines.push(line);
    }

    let count = lines.len();
    for l in lines {
        let _ = writeln!(out, "{l}");
    }
    count
}

fn format_verdict(
    spec: &TaskSpec,
    outcome: Outcome,
    duration: Duration,
    exit_code: i32,
    gutter: usize,
    s: &Styles,
) -> String {
    let color = s.task_color(spec.color_idx);
    let name_padded = spec.name.pad_right(gutter);

    let icon = match outcome {
        Outcome::Passed => format!("{}✔{}", s.green(), s.reset()),
        Outcome::Failed(_) => format!("{}✖{}", s.red(), s.reset()),
        Outcome::Timeout => format!("{}⏱{}", s.yellow(), s.reset()),
        Outcome::Cancelled => format!("{}⊘{}", s.gray(), s.reset()),
        Outcome::Running | Outcome::Pending => " ".to_string(),
    };

    let detail = match outcome {
        Outcome::Passed => format!("{}passed{}", s.green(), s.reset()),
        Outcome::Failed(_) => {
            format!("{}failed (exit code {}){}", s.red(), exit_code, s.reset())
        }
        Outcome::Timeout => format!("{}timed out{}", s.yellow(), s.reset()),
        Outcome::Cancelled => format!("{}cancelled{}", s.gray(), s.reset()),
        _ => String::new(),
    };

    format!(
        "{} {}{}{} {} {}({}){}\n",
        icon,
        color,
        name_padded,
        s.reset(),
        detail,
        s.gray(),
        format_duration(duration),
        s.reset()
    )
}

fn print_verdict_summary(results: &[TaskResult], elapsed: Duration, gutter: usize, s: &Styles) {
    println!();
    println!("{}Summary:{}", s.bold(), s.reset());

    let mut ranked = results.to_vec();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.duration));

    let max_dur = ranked
        .first()
        .map(|r| r.duration.as_secs_f64())
        .unwrap_or(1.0)
        .max(0.001);
    let term_width = terminal_columns();
    let max_bar_width = 30.min(term_width.saturating_sub(gutter + 32)).max(5);

    let failed: Vec<&TaskResult> = ranked.iter().filter(|r| r.outcome.is_bad()).collect();
    let passed = ranked
        .iter()
        .filter(|r| r.outcome == Outcome::Passed)
        .count();
    let cancelled = ranked
        .iter()
        .filter(|r| r.outcome == Outcome::Cancelled)
        .count();

    let mut failed_names = String::new();
    for r in &failed {
        failed_names.push_str(", ");
        failed_names.push_str(&r.name);
    }

    for r in &ranked {
        let color = s.task_color(r.color_idx);
        let name_padded = r.name.pad_right(gutter);

        let icon = match r.outcome {
            Outcome::Passed => format!("{}✔{}", s.green(), s.reset()),
            Outcome::Failed(_) => format!("{}✖{}", s.red(), s.reset()),
            Outcome::Timeout => format!("{}⏱{}", s.yellow(), s.reset()),
            Outcome::Cancelled => format!("{}⊘{}", s.gray(), s.reset()),
            _ => " ".to_string(),
        };

        let dur_fraction = r.duration.as_secs_f64() / max_dur;
        let bar_len = ((dur_fraction * max_bar_width as f64).round() as usize).max(1);

        let bar = if r.outcome == Outcome::Cancelled {
            format!("{}{}{}", s.gray(), "╌".repeat(bar_len), s.reset())
        } else {
            let bar_color = match r.outcome {
                Outcome::Passed => s.green(),
                Outcome::Failed(_) => s.red(),
                Outcome::Timeout => s.yellow(),
                _ => s.gray(),
            };
            format!("{}{}{}", bar_color, "█".repeat(bar_len), s.reset())
        };

        println!(
            "  {} {}{}{} {}({}){} {}",
            icon,
            color,
            name_padded,
            s.reset(),
            s.gray(),
            format_duration(r.duration).pad_left(7),
            s.reset(),
            bar
        );
    }

    println!();
    if failed.is_empty() && cancelled == 0 {
        println!(
            "{}{} ✔ {}/{} passed{} {}in {}{}",
            s.green(),
            s.bold(),
            results.len(),
            results.len(),
            s.reset(),
            s.gray(),
            format_duration(elapsed),
            s.reset()
        );
    } else {
        let cancel_part = if cancelled > 0 {
            format!(", {} cancelled", cancelled)
        } else {
            String::new()
        };
        println!(
            "{}{} ✖ {} failed{}{}, {} passed{} {}in {}{}",
            s.red(),
            s.bold(),
            failed.len(),
            s.reset(),
            cancel_part,
            passed,
            s.reset(),
            s.gray(),
            format_duration(elapsed),
            s.reset()
        );
        if !failed.is_empty() {
            println!(
                "{}   ↳{} {}{}{}",
                s.red(),
                s.reset(),
                s.gray(),
                &failed_names[3..],
                s.reset()
            );
        }
    }

    // Turborepo-inspired concurrency efficiency report
    let total_cpu_duration: Duration = results.iter().map(|r| r.duration).sum();
    if total_cpu_duration > elapsed && elapsed.as_millis() > 30 && results.len() > 1 {
        let speedup = total_cpu_duration.as_secs_f64() / elapsed.as_secs_f64().max(0.001);
        let saved = total_cpu_duration.saturating_sub(elapsed);
        println!(
            "{}⚡ Concurrency: {:.1}x speedup (saved {} across {} tasks){}",
            s.cyan(),
            speedup,
            format_duration(saved),
            results.len(),
            s.reset()
        );
    }

    println!();
}

trait PadExt {
    fn pad_right(&self, width: usize) -> String;
    fn pad_left(&self, width: usize) -> String;
}

impl PadExt for str {
    fn pad_right(&self, width: usize) -> String {
        let w = visible_width(self);
        if w >= width {
            self.to_string()
        } else {
            format!("{}{}", self, " ".repeat(width - w))
        }
    }

    fn pad_left(&self, width: usize) -> String {
        let w = visible_width(self);
        if w >= width {
            self.to_string()
        } else {
            format!("{}{}", " ".repeat(width - w), self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_runner_bin() {
        #[cfg(not(windows))]
        {
            assert_eq!(resolve_runner_bin("npm"), "npm");
            assert_eq!(resolve_runner_bin("cargo"), "cargo");
        }
        #[cfg(windows)]
        {
            assert_eq!(resolve_runner_bin("npm.cmd"), "npm.cmd");
            assert_eq!(resolve_runner_bin("cargo.exe"), "cargo.exe");
        }
    }
}
