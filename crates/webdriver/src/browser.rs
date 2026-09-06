//! Browser process management, persistent sessions, and DevTools discovery.

use crate::cdp::{CdpSession, WebSocketClient};
use crate::json;
use local_common::paths::tool_data_dir;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub struct BrowserInstance {
    child: Child,
    user_data_dir: PathBuf,
    is_persistent: bool,
}

impl BrowserInstance {
    pub fn launch(
        custom_browser: Option<&str>,
        headless: bool,
        user_data_dir: Option<PathBuf>,
        extra_args: &[String],
    ) -> Result<(Self, CdpSession), String> {
        let browser_path = find_browser(custom_browser)?;

        let (data_dir, is_persistent) = match user_data_dir {
            Some(dir) => {
                fs::create_dir_all(&dir).map_err(|e| {
                    format!("failed to create session directory {}: {e}", dir.display())
                })?;
                (dir, true)
            }
            None => {
                let temp_dir = env::temp_dir().join(format!(
                    "belt-webdriver-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0)
                ));
                fs::create_dir_all(&temp_dir)
                    .map_err(|e| format!("failed to create temp profile dir: {e}"))?;
                (temp_dir, false)
            }
        };

        let mut cmd = Command::new(&browser_path);

        if headless {
            cmd.arg("--headless=new");
        }

        cmd.arg("--remote-debugging-port=0")
            .arg("--disable-gpu")
            .arg("--hide-scrollbars")
            .arg("--mute-audio")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(format!("--user-data-dir={}", data_dir.display()));

        // Chrome's sandbox cannot start as root, and a container's default 64 MB
        // /dev/shm is too small for it — so a CI image needs both of these or the
        // browser dies before DevTools ever comes up.
        if running_as_root() {
            cmd.arg("--no-sandbox").arg("--disable-dev-shm-usage");
        }

        for arg in extra_args {
            cmd.arg(arg);
        }

        cmd.arg("about:blank")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        // Chrome forks renderer/GPU helper subprocesses; put it in its own
        // process group so Drop can sweep away the whole tree, not just the
        // top-level process.
        local_common::process::own_process_group(&mut cmd);

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "failed to launch browser at {}: {e}",
                browser_path.display()
            )
        })?;

        let setup = (|| -> Result<CdpSession, String> {
            let stderr = child
                .stderr
                .take()
                .ok_or("failed to capture browser stderr")?;
            let (tx, rx) = mpsc::channel();

            // Read stderr in background to capture DevTools listening port.
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if let Some(pos) = line.find("DevTools listening on ws://") {
                        let rest = &line[pos + "DevTools listening on ws://".len()..];
                        let _ = tx.send(rest.to_string());
                        break;
                    }
                }
            });

            let ws_endpoint = rx
                .recv_timeout(Duration::from_secs(15))
                .map_err(|_| "timed out waiting for browser DevTools endpoint".to_string())?;

            // Format is e.g. "127.0.0.1:54321/devtools/browser/uuid"
            let parts: Vec<&str> = ws_endpoint.splitn(2, '/').collect();
            let addr = parts[0];
            let port: u16 = addr
                .split(':')
                .nth(1)
                .and_then(|p| p.parse().ok())
                .ok_or_else(|| format!("invalid DevTools address '{addr}'"))?;

            let page_ws_path = create_new_tab(port)?;
            let ws_client = WebSocketClient::connect("127.0.0.1", port, &page_ws_path)?;
            let mut session = CdpSession::new(ws_client);
            session.enable_domains()?;
            Ok(session)
        })();

        match setup {
            Ok(session) => Ok((
                Self {
                    child,
                    user_data_dir: data_dir,
                    is_persistent,
                },
                session,
            )),
            Err(error) => {
                local_common::process::terminate(&mut child);
                if !is_persistent {
                    let _ = fs::remove_dir_all(&data_dir);
                }
                Err(error)
            }
        }
    }
}

impl BrowserInstance {
    /// Shut the browser down cleanly so a persistent profile survives.
    ///
    /// Chrome writes cookies and localStorage to disk on exit; killing it
    /// outright loses everything the run just authenticated. Ephemeral profiles
    /// are thrown away anyway, so they skip the wait.
    pub fn shutdown(&mut self, session: &mut CdpSession) {
        if !self.is_persistent {
            return;
        }

        let _ = session.close_browser();
        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                _ => return,
            }
        }
    }
}

impl Drop for BrowserInstance {
    fn drop(&mut self) {
        local_common::process::terminate(&mut self.child);
        if !self.is_persistent {
            let _ = fs::remove_dir_all(&self.user_data_dir);
        }
    }
}

/// Whether this process is root, which is the signature of a CI container.
///
/// Read from the owner of `/proc/self` rather than a `geteuid` binding, since
/// this crate carries no dependencies. Non-Linux systems have no `/proc` and
/// report `false` — where the sandbox flags are not needed anyway.
fn running_as_root() -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata("/proc/self")
            .map(|m| m.uid() == 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

pub fn resolve_session_path(session_name: &str) -> Result<PathBuf, String> {
    let sanitized: String = session_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    if sanitized.is_empty() {
        return Err("session name cannot be empty".to_string());
    }

    let base = tool_data_dir("webdriver")
        .ok_or_else(|| "failed to resolve user data directory".to_string())?;

    Ok(base.join("sessions").join(sanitized))
}

pub fn list_sessions() -> Result<Vec<(String, u64)>, String> {
    let base = tool_data_dir("webdriver")
        .ok_or_else(|| "failed to resolve user data directory".to_string())?;
    let sessions_dir = base.join("sessions");

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut list = Vec::new();
    let entries = fs::read_dir(&sessions_dir)
        .map_err(|e| format!("failed to read sessions directory: {e}"))?;

    for entry in entries.flatten() {
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                let size = dir_size(&entry.path());
                list.push((name, size));
            }
        }
    }

    list.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(list)
}

pub fn clear_session(session_name: &str) -> Result<PathBuf, String> {
    let path = resolve_session_path(session_name)?;
    if path.exists() {
        fs::remove_dir_all(&path)
            .map_err(|e| format!("failed to remove session directory {}: {e}", path.display()))?;
    }
    Ok(path)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn http_request(port: u16, method: &str, path: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| format!("failed to connect to 127.0.0.1:{port}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;

    let req = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Content-Length: 0\r\n\
         Connection: close\r\n\
         \r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let mut content_length: Option<usize> = None;

    // Read header
    while buf.len() < 8192 {
        if stream.read_exact(&mut byte).is_err() {
            break;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            let header_str = String::from_utf8_lossy(&buf);
            for line in header_str.lines() {
                if let Some(val) = line.to_lowercase().strip_prefix("content-length:") {
                    if let Ok(len) = val.trim().parse::<usize>() {
                        content_length = Some(len);
                    }
                }
            }
            break;
        }
    }

    // Read body
    let mut body = Vec::new();
    if let Some(len) = content_length {
        body.resize(len, 0);
        stream
            .read_exact(&mut body)
            .map_err(|e| format!("failed to read HTTP body of {len} bytes: {e}"))?;
    } else {
        let mut rest = Vec::new();
        let _ = stream.read_to_end(&mut rest);
        body.extend_from_slice(&rest);
    }

    String::from_utf8(body).map_err(|e| format!("invalid UTF-8 in response body: {e}"))
}

fn create_new_tab(port: u16) -> Result<String, String> {
    let start = Instant::now();
    let timeout = Duration::from_secs(5);

    while start.elapsed() < timeout {
        if let Ok(body) = http_request(port, "PUT", "/json/new") {
            if let Ok(val) = json::parse(&body) {
                if let Some(ws_url) = val.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                    if let Some(pos) = ws_url.find("/devtools/page/") {
                        return Ok(ws_url[pos..].to_string());
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    // Fallback: query /json/list
    if let Ok(body) = http_request(port, "GET", "/json/list") {
        if let Ok(val) = json::parse(&body) {
            if let Some(arr) = val.as_array() {
                for item in arr {
                    if item.get("type").and_then(|t| t.as_str()) == Some("page") {
                        if let Some(ws_url) =
                            item.get("webSocketDebuggerUrl").and_then(|v| v.as_str())
                        {
                            if let Some(pos) = ws_url.find("/devtools/page/") {
                                return Ok(ws_url[pos..].to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    Err("failed to obtain page target WebSocket URL from browser".to_string())
}

pub fn find_browser(custom: Option<&str>) -> Result<PathBuf, String> {
    if let Some(c) = custom {
        let p = PathBuf::from(c);
        if p.exists() {
            return Ok(p);
        }
        return Err(format!("specified browser binary does not exist: {c}"));
    }

    if let Ok(env_bin) = env::var("CHROME_BIN").or_else(|_| env::var("BROWSER_BIN")) {
        let p = PathBuf::from(env_bin);
        if p.exists() {
            return Ok(p);
        }
    }

    let candidates = [
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/brave-browser",
        "/usr/bin/microsoft-edge",
        "/usr/bin/microsoft-edge-stable",
        "/snap/bin/chromium",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
        r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
    ];

    for path in candidates {
        let p = Path::new(path);
        if p.exists() {
            return Ok(p.to_path_buf());
        }
    }

    for pf_var in &["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Ok(pf) = env::var(pf_var) {
            let pf_dir = PathBuf::from(pf);
            for rel in &[
                r"Google\Chrome\Application\chrome.exe",
                r"Microsoft\Edge\Application\msedge.exe",
                r"BraveSoftware\Brave-Browser\Application\brave.exe",
            ] {
                let full = pf_dir.join(rel);
                if full.exists() {
                    return Ok(full);
                }
            }
        }
    }

    if let Ok(local_app_data) = env::var("LOCALAPPDATA") {
        let local_app_data_dir = PathBuf::from(local_app_data);
        for rel in &[
            r"Google\Chrome\Application\chrome.exe",
            r"Microsoft\Edge\Application\msedge.exe",
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
        ] {
            let full = local_app_data_dir.join(rel);
            if full.exists() {
                return Ok(full);
            }
        }
    }

    // Check PATH
    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            for bin_name in &[
                "chromium",
                "chromium-browser",
                "google-chrome",
                "google-chrome-stable",
                "brave-browser",
                "microsoft-edge",
                "microsoft-edge-stable",
                "chrome",
                "msedge",
                "brave",
            ] {
                let full = dir.join(bin_name);
                if full.is_file() {
                    return Ok(full);
                }
                let full_exe = dir.join(format!("{bin_name}.exe"));
                if full_exe.is_file() {
                    return Ok(full_exe);
                }
            }
        }
    }

    Err(
        "no Chromium-based browser found. Install Chromium, Google Chrome, Edge, or Brave, or set CHROME_BIN."
            .to_string(),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn launch_failure_reaps_the_browser_process() {
        let tmp =
            std::env::temp_dir().join(format!("webdriver-launch-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let pid_file = tmp.join("browser.pid");
        let browser = tmp.join("fake-browser");
        fs::write(
            &browser,
            format!(
                "#!/bin/sh\necho $$ > '{}'\necho 'DevTools listening on ws://invalid' >&2\nsleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&browser, fs::Permissions::from_mode(0o755)).unwrap();

        let result = BrowserInstance::launch(Some(browser.to_str().unwrap()), true, None, &[]);
        assert!(result.is_err());

        let pid = fs::read_to_string(&pid_file).unwrap();
        let status = Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap();
        assert!(!status.success(), "failed launch left its browser running");

        let _ = fs::remove_dir_all(&tmp);
    }
}
