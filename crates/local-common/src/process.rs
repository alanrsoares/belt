//! Process-group lifecycle helpers for tools that spawn subprocesses which
//! may themselves fork children (browsers, package managers, build tools, …).
//!
//! Pair [`own_process_group`] at spawn time with [`terminate`] at cleanup
//! time so cancellation and error paths reap the whole subprocess tree, not
//! just the top-level process.

use std::process::{Child, Command};

/// Put `cmd` in its own process group so [`terminate`] can sweep
/// away its whole subprocess tree rather than just the top-level process.
pub fn own_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_PROCESS_GROUP = 0x00000200
        cmd.creation_flags(0x00000200);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = cmd;
    }
}

/// Force-terminate `child` and its whole process tree — so
/// subprocesses forked by a `child` spawned via [`own_process_group`] don't
/// outlive it.
pub fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        // A negative PID targets the process group created at spawn.
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{}", child.id())])
            .status();
    }
    #[cfg(windows)]
    {
        // /F = force, /T = terminate process tree
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &child.id().to_string()])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
