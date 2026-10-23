//! X-display management: spawn a private `Xvfb`, wait for its socket, and
//! tear it down on exit.
//!
//! By default we spawn a fresh `Xvfb` on a free display number in `1..=99`
//! (a number with no lingering socket file). `--display :N` instead reuses an
//! existing display — we trust the socket is already there and never spawn.

use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

use local_common::process::own_process_group;

/// A live display handle. Either we spawned Xvfb (`Owned`) or we reused one
/// (`Shared`).
pub enum Display {
    Owned { child: Child, number: u16 },
    Shared { number: u16 },
}

impl Display {
    pub fn number(&self) -> u16 {
        match self {
            Self::Owned { number, .. } | Self::Shared { number } => *number,
        }
    }

    pub fn is_shared(&self) -> bool {
        matches!(self, Self::Shared { .. })
    }

    /// Kill our Xvfb (if we own one). No-op for shared displays.
    pub fn close(&mut self) {
        if let Self::Owned { child, .. } = self {
            local_common::process::terminate(child);
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        self.close();
    }
}

/// The X11 unix socket for `number` (`/tmp/.X11-unix/X<n>` on Linux).
fn socket_path(number: u16) -> PathBuf {
    PathBuf::from(format!("/tmp/.X11-unix/X{number}"))
}

fn socket_ready(number: u16) -> bool {
    socket_path(number).exists()
}

fn poll_socket(child: &mut Child, number: u16, wait: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < wait {
        if socket_ready(number) {
            return true;
        }
        if let Ok(Some(_)) = child.try_wait() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    socket_ready(number)
}

/// First display number in `1..=99` with no lingering socket file that hasn't been tried yet.
fn pick_free_number(tried: &[u16]) -> Option<u16> {
    (1..=99).find(|n| !tried.contains(n) && !socket_ready(*n))
}

/// Reuse `pinned` if given, otherwise spawn a fresh Xvfb on a free number.
///
/// If a spawned Xvfb's socket never appears within `wait`, we kill that
/// attempt and try a few other numbers before giving up (a zombie display
/// can hold a number without a socket).
pub fn start(pinned: Option<u16>, screen: (u16, u16), wait: Duration) -> Result<Display, String> {
    if let Some(n) = pinned {
        if !socket_ready(n) {
            return Err(format!(
                "--display :{n} given but no socket at {} (is a display running there?)",
                socket_path(n).display()
            ));
        }
        return Ok(Display::Shared { number: n });
    }

    let screen_arg = format!("{}x{}x24", screen.0, screen.1);
    let mut tried: Vec<u16> = Vec::new();
    let mut last_err = String::new();

    for _attempt in 0..5 {
        let n = match pick_free_number(&tried) {
            Some(n) => n,
            None => {
                last_err = "no free display numbers (1..99) found".into();
                break;
            }
        };
        tried.push(n);

        let mut cmd = std::process::Command::new("Xvfb");
        cmd.arg(format!(":{n}"))
            .args(["-screen", "0", &screen_arg, "-nolisten", "tcp"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        own_process_group(&mut cmd);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                last_err = format!("failed to spawn Xvfb :{n}: {e}. Is Xvfb installed?");
                continue;
            }
        };

        if poll_socket(&mut child, n, wait) {
            return Ok(Display::Owned { child, number: n });
        }

        // Socket never came up — kill this attempt before the next.
        child.kill().ok();
        let _ = child.wait();
        last_err = format!("Xvfb :{n} spawned but socket never appeared in {wait:?}");
    }

    Err(format!(
        "could not start a virtual display (last error: {last_err}). \
         Is Xvfb installed? Try `--display :0` to reuse your real screen."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_paths_for_known_numbers() {
        // Pure path construction — no display required, safe on any OS.
        assert!(socket_path(0).ends_with(".X11-unix/X0"));
        assert!(socket_path(73).ends_with(".X11-unix/X73"));
        assert!(socket_path(99).ends_with(".X11-unix/X99"));
    }

    #[test]
    fn pick_free_number_skips_tried() {
        let tried = vec![1, 2, 3];
        if let Some(n) = pick_free_number(&tried) {
            assert!(!tried.contains(&n));
        }
    }
}
