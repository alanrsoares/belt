//! Stable-state capture loop.
//!
//! Hardened (Tor-lineage) browsers expose no "page finished" signal without
//! CDP, so the only reliable settled-detector is pixel stability: two
//! consecutive identical captures mean nothing in the window is changing any
//! more. A hard budget caps everything so a perpetually animating page cannot
//! run forever.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::term::{Color, Out};

/// Capture the root window of `display` to `out_path`.
///
/// Returns the written byte length so the caller can flag near-blank grabs.
pub fn capture_root(display: u16, out_path: &Path) -> Result<u64, String> {
    let output = std::process::Command::new("import")
        .args(["-window", "root", "-silent"])
        .arg(out_path)
        .env("DISPLAY", format!(":{display}"))
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "failed to run import: command not found (is ImageMagick installed?)".to_string()
            } else {
                format!("failed to run import: {e}")
            }
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("import failed: {}", stderr.trim()));
    }
    let len = fs::metadata(out_path).map(|m| m.len()).map_err(|e| {
        format!(
            "capture written but unreadable ({e}): {}",
            out_path.display()
        )
    })?;
    Ok(len)
}

/// Byte-identical? A sound settled-proxy for a deterministic grabber: two
/// identical captures of an unchanging screen are identical PNGs. Errors
/// propagate rather than assume identity.
pub fn identical(a: &Path, b: &Path) -> Result<bool, String> {
    let meta_a = fs::metadata(a).map_err(|e| format!("cannot re-read {}: {e}", a.display()))?;
    let meta_b = fs::metadata(b).map_err(|e| format!("cannot re-read {}: {e}", b.display()))?;
    if meta_a.len() != meta_b.len() {
        return Ok(false);
    }
    let x = fs::read(a).map_err(|e| format!("cannot re-read {}: {e}", a.display()))?;
    let y = fs::read(b).map_err(|e| format!("cannot re-read {}: {e}", b.display()))?;
    Ok(x == y)
}

/// Budget bounds for the settle-and-capture phase, grouped so
/// [`wait_stable`] stays under clippy's argument count.
pub struct Budget {
    /// Delay before the first capture, letting the browser load and paint.
    pub settle: Duration,
    /// Instant the browser was launched (for the "stable after Xs" line).
    pub launch: Instant,
    /// Hard deadline for the whole settle-and-capture phase.
    pub deadline: Instant,
}

/// Wait out `budget.settle` (where the browser actually loads and paints),
/// then capture pairs of frames until two agree or `budget.deadline` is
/// exhausted.
///
/// `curr` receives the final, winning frame. `prev` is a scratch companion
/// path owned by this call; it is deleted before returning.
pub fn wait_stable(
    out: &mut Out,
    c: &Color,
    display: u16,
    curr: &Path,
    prev: &Path,
    budget: &Budget,
) -> Result<(), String> {
    let start = Instant::now();
    std::thread::sleep(budget.settle);
    out.step(format!(
        "settle {:.0}s elapsed",
        budget.settle.as_secs_f64().round()
    ));

    let beat = Duration::from_millis(750);
    let mut attempt = 0u32;

    loop {
        attempt += 1;
        capture_root(display, curr)?;

        if attempt >= 2 && prev.exists() && identical(curr, prev)? {
            let total = budget.launch.elapsed().as_secs_f64();
            out.result(format!(
                "{} stable after {total:.1}s (2 identical captures)",
                c.cyan("✓")
            ));
            let _ = fs::remove_file(prev);
            return Ok(());
        }

        // Not yet settled: copy curr to prev for next round's comparison.
        // Copying preserves curr on disk so that even on deadline expiry,
        // curr remains the valid most-recent capture.
        if let Err(e) = fs::copy(curr, prev) {
            let _ = fs::remove_file(prev);
            return Err(format!("cannot rotate capture files: {e}"));
        }

        if Instant::now() >= budget.deadline {
            break;
        }
        std::thread::sleep(beat);
    }

    out.warn(format!(
        "{} page did not settle within {:.0}s; keeping last capture",
        c.yellow("⚠"),
        start.elapsed().as_secs_f64().round()
    ));
    let _ = fs::remove_file(prev);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_files() {
        let dir = std::env::temp_dir().join(format!("grabsite-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let a = dir.join("a.png");
        let b = dir.join("b.png");
        let c = dir.join("c.png");

        fs::write(&a, b"test-data-123").unwrap();
        fs::write(&b, b"test-data-123").unwrap();
        fs::write(&c, b"test-data-diff").unwrap();

        assert!(identical(&a, &b).unwrap());
        assert!(!identical(&a, &c).unwrap());

        let non_existent = dir.join("nope.png");
        assert!(identical(&a, &non_existent).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
