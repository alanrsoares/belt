//! Zero-dependency Chrome DevTools Protocol (CDP) WebSocket client.

use crate::base64;
use crate::json::{self, JsonValue};
use crate::locator;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub struct WebSocketClient {
    stream: TcpStream,
}

impl WebSocketClient {
    pub fn connect(host: &str, port: u16, path: &str) -> Result<Self, String> {
        let stream = TcpStream::connect((host, port))
            .map_err(|e| format!("failed to connect to CDP at {host}:{port}: {e}"))?;

        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .map_err(|e| format!("failed to set read timeout: {e}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(60)))
            .map_err(|e| format!("failed to set write timeout: {e}"))?;

        let mut client = Self { stream };
        client.handshake(host, port, path)?;
        Ok(client)
    }

    fn handshake(&mut self, host: &str, port: u16, path: &str) -> Result<(), String> {
        let request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\
             \r\n"
        );

        self.stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("failed to send websocket handshake: {e}"))?;
        self.stream
            .flush()
            .map_err(|e| format!("failed to flush handshake: {e}"))?;

        // Read headers until \r\n\r\n
        let mut header_buf = Vec::new();
        let mut byte = [0u8; 1];

        while header_buf.len() < 8192 {
            self.stream
                .read_exact(&mut byte)
                .map_err(|e| format!("failed to read handshake response: {e}"))?;
            header_buf.push(byte[0]);

            if header_buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }

        if !header_buf.ends_with(b"\r\n\r\n") {
            return Err("websocket handshake headers exceeded 8192 bytes".to_string());
        }

        let resp_str = String::from_utf8_lossy(&header_buf);
        if !resp_str.starts_with("HTTP/1.1 101") && !resp_str.starts_with("HTTP/1.0 101") {
            return Err(format!("invalid websocket handshake response: {resp_str}"));
        }

        Ok(())
    }

    pub fn send_text(&mut self, text: &str) -> Result<(), String> {
        let payload = text.as_bytes();
        let mut frame = Vec::with_capacity(payload.len() + 14);

        // Byte 0: FIN (0x80) | Text Opcode (0x01)
        frame.push(0x81);

        // Mask bit is required from client to server (0x80)
        let len = payload.len();
        if len <= 125 {
            frame.push(0x80 | (len as u8));
        } else if len <= 65535 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }

        // 4-byte mask key
        let mask = [0x12, 0x34, 0x56, 0x78];
        frame.extend_from_slice(&mask);

        // Masked payload
        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i % 4]);
        }

        self.stream
            .write_all(&frame)
            .map_err(|e| format!("failed to send websocket frame: {e}"))?;
        self.stream
            .flush()
            .map_err(|e| format!("failed to flush websocket frame: {e}"))?;

        Ok(())
    }

    pub fn read_text(&mut self) -> Result<String, String> {
        const MAX_FRAME_LEN: usize = 256 * 1024 * 1024;
        let mut full_payload = Vec::new();

        loop {
            let mut header = [0u8; 2];
            self.stream
                .read_exact(&mut header)
                .map_err(|e| format!("failed to read websocket frame header: {e}"))?;

            let fin = (header[0] & 0x80) != 0;
            let opcode = header[0] & 0x0f;
            let masked = (header[1] & 0x80) != 0;
            let len_code = header[1] & 0x7f;

            let len: usize = if len_code <= 125 {
                len_code as usize
            } else if len_code == 126 {
                let mut b = [0u8; 2];
                self.stream
                    .read_exact(&mut b)
                    .map_err(|e| format!("failed to read 16-bit length: {e}"))?;
                u16::from_be_bytes(b) as usize
            } else {
                let mut b = [0u8; 8];
                self.stream
                    .read_exact(&mut b)
                    .map_err(|e| format!("failed to read 64-bit length: {e}"))?;
                usize::try_from(u64::from_be_bytes(b))
                    .map_err(|_| "websocket frame length overflows usize".to_string())?
            };

            if len > MAX_FRAME_LEN {
                return Err(format!(
                    "websocket frame length {len} exceeds maximum allowed ({MAX_FRAME_LEN} bytes)"
                ));
            }

            let mask_key = if masked {
                let mut m = [0u8; 4];
                self.stream
                    .read_exact(&mut m)
                    .map_err(|e| format!("failed to read mask key: {e}"))?;
                Some(m)
            } else {
                None
            };

            let mut payload = vec![0u8; len];
            self.stream
                .read_exact(&mut payload)
                .map_err(|e| format!("failed to read frame payload of {len} bytes: {e}"))?;

            if let Some(mask) = mask_key {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }

            match opcode {
                // Continuation frame
                0x0 => {
                    full_payload.extend_from_slice(&payload);
                }
                // Text frame
                0x1 => {
                    full_payload.extend_from_slice(&payload);
                }
                // Close frame
                0x8 => {
                    return Err("websocket connection closed by peer".to_string());
                }
                // Ping frame -> send Pong
                0x9 => {
                    let pong_payload = &payload[..payload.len().min(125)];
                    let mut pong = Vec::with_capacity(pong_payload.len() + 6);
                    pong.push(0x8A); // FIN | Pong
                    pong.push(0x80 | (pong_payload.len() as u8));
                    let pmask = [0x00, 0x00, 0x00, 0x00];
                    pong.extend_from_slice(&pmask);
                    pong.extend_from_slice(pong_payload);
                    let _ = self.stream.write_all(&pong);
                    let _ = self.stream.flush();
                    continue;
                }
                // Pong frame -> ignore
                0xA => continue,
                other => {
                    return Err(format!("unsupported websocket opcode 0x{other:02X}"));
                }
            }

            if fin {
                break;
            }
        }

        String::from_utf8(full_payload).map_err(|e| format!("invalid UTF-8 in message: {e}"))
    }
}

/// A console message or uncaught exception captured from the page.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
}

impl ConsoleEntry {
    pub fn is_error(&self) -> bool {
        matches!(self.level.as_str(), "error" | "warning" | "exception")
    }
}

/// A JS execution context, one per frame. Tracked so commands can be aimed at
/// an iframe instead of always hitting the top frame.
#[derive(Debug, Clone)]
struct ExecutionContext {
    id: i64,
    frame_id: String,
}

/// One interactive node from the accessibility tree, addressable as `ref=eN`.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotNode {
    pub reference: String,
    pub role: String,
    pub name: String,
    pub backend_node_id: i64,
}

pub struct CdpSession {
    ws: WebSocketClient,
    next_id: u64,
    default_timeout_ms: u64,
    console: Vec<ConsoleEntry>,
    contexts: Vec<ExecutionContext>,
    /// Frame the next command targets; `None` means the top frame.
    target_frame: Option<String>,
    /// Last `snapshot` result, keyed by the `eN` reference handed to the agent.
    refs: HashMap<String, i64>,
}

impl CdpSession {
    pub fn new(ws: WebSocketClient) -> Self {
        Self {
            ws,
            next_id: 1,
            default_timeout_ms: 30_000,
            console: Vec::new(),
            contexts: Vec::new(),
            target_frame: None,
            refs: HashMap::new(),
        }
    }

    pub fn set_default_timeout(&mut self, timeout_ms: u64) {
        self.default_timeout_ms = timeout_ms;
    }

    /// Console entries captured so far, oldest first.
    pub fn console_entries(&self) -> &[ConsoleEntry] {
        &self.console
    }

    pub fn clear_console(&mut self) {
        self.console.clear();
    }

    /// Pump the socket so events emitted since the last command land in the
    /// buffer. CDP delivers events before the reply to a later command, so a
    /// trivial round-trip is enough to drain them.
    pub fn drain_events(&mut self) -> Result<(), String> {
        self.evaluate("0")?;
        Ok(())
    }

    fn record_event(&mut self, val: &JsonValue) {
        let Some(method) = val.get("method").and_then(|m| m.as_str()) else {
            return;
        };
        let params = val.get("params");

        match method {
            "Runtime.executionContextCreated" => {
                let ctx = params.and_then(|p| p.get("context"));
                let id = ctx.and_then(|c| c.get("id")).and_then(|i| i.as_i64());
                let frame_id = ctx
                    .and_then(|c| c.get("auxData"))
                    .and_then(|a| a.get("frameId"))
                    .and_then(|f| f.as_str());
                if let (Some(id), Some(frame_id)) = (id, frame_id) {
                    self.contexts.retain(|c| c.id != id);
                    self.contexts.push(ExecutionContext {
                        id,
                        frame_id: frame_id.to_string(),
                    });
                }
            }
            "Runtime.executionContextDestroyed" => {
                if let Some(id) = params
                    .and_then(|p| p.get("executionContextId"))
                    .and_then(|i| i.as_i64())
                {
                    self.contexts.retain(|c| c.id != id);
                }
            }
            "Runtime.executionContextsCleared" => self.contexts.clear(),
            _ => {}
        }

        let entry = match method {
            "Runtime.consoleAPICalled" => {
                let level = params
                    .and_then(|p| p.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("log")
                    .to_string();
                let text = params
                    .and_then(|p| p.get("args"))
                    .and_then(|a| a.as_array())
                    .map(|args| {
                        args.iter()
                            .map(describe_remote_object)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                Some(ConsoleEntry { level, text })
            }
            "Runtime.exceptionThrown" => {
                let details = params.and_then(|p| p.get("exceptionDetails"));
                let text = details
                    .and_then(|d| d.get("exception"))
                    .map(describe_remote_object)
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        details
                            .and_then(|d| d.get("text"))
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| "uncaught exception".to_string());
                Some(ConsoleEntry {
                    level: "exception".to_string(),
                    text,
                })
            }
            "Log.entryAdded" => {
                let entry = params.and_then(|p| p.get("entry"));
                let level = entry
                    .and_then(|e| e.get("level"))
                    .and_then(|l| l.as_str())
                    .unwrap_or("info")
                    .to_string();
                let text = entry
                    .and_then(|e| e.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(ConsoleEntry { level, text })
            }
            _ => None,
        };

        if let Some(entry) = entry {
            if !entry.text.is_empty() {
                self.console.push(entry);
            }
        }
    }

    pub fn call(&mut self, method: &str, params: JsonValue) -> Result<JsonValue, String> {
        let req_id = self.next_id;
        self.next_id += 1;

        let mut map = HashMap::new();
        map.insert("id".to_string(), JsonValue::Number(req_id as f64));
        map.insert("method".to_string(), JsonValue::String(method.to_string()));
        map.insert("params".to_string(), params);

        let req_json = JsonValue::Object(map).to_json_string();
        self.ws.send_text(&req_json)?;

        // Wait for response matching req_id
        let start = Instant::now();
        let timeout = Duration::from_secs(60);

        loop {
            if start.elapsed() > timeout {
                return Err(format!("timeout waiting for CDP response to '{method}'"));
            }

            let msg = self.ws.read_text()?;
            let val = json::parse(&msg)?;
            self.record_event(&val);

            if let Some(id_val) = val.get("id") {
                if id_val.as_i64() == Some(req_id as i64) {
                    if let Some(err) = val.get("error") {
                        let msg = err
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown CDP error");
                        return Err(format!("CDP {method} error: {msg}"));
                    }
                    if let Some(result) = val.get("result") {
                        return Ok(result.clone());
                    }
                    return Ok(JsonValue::Null);
                }
            }
        }
    }

    pub fn enable_domains(&mut self) -> Result<(), String> {
        self.call("Page.enable", JsonValue::Object(HashMap::new()))?;
        self.call("Runtime.enable", JsonValue::Object(HashMap::new()))?;
        self.call("DOM.enable", JsonValue::Object(HashMap::new()))?;
        // Log domain surfaces network/security failures the page never logs itself.
        let _ = self.call("Log.enable", JsonValue::Object(HashMap::new()));
        let _ = self.call("Accessibility.enable", JsonValue::Object(HashMap::new()));
        Ok(())
    }

    pub fn navigate(&mut self, url: &str, timeout_ms: u64) -> Result<(), String> {
        let mut params = HashMap::new();
        params.insert("url".to_string(), JsonValue::String(url.to_string()));
        self.call("Page.navigate", JsonValue::Object(params))?;

        self.await_ready(timeout_ms);
        Ok(())
    }

    /// Poll `document.readyState` until the document is usable. Returns whether
    /// it got there; callers proceed regardless, matching prior behaviour.
    fn await_ready(&mut self, timeout_ms: u64) -> bool {
        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            if let Ok(val) = self.evaluate("document.readyState") {
                if matches!(val.as_str(), Some("interactive") | Some("complete")) {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        false
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        let mut params = HashMap::new();
        params.insert("width".to_string(), JsonValue::Number(width as f64));
        params.insert("height".to_string(), JsonValue::Number(height as f64));
        params.insert("deviceScaleFactor".to_string(), JsonValue::Number(1.0));
        params.insert("mobile".to_string(), JsonValue::Bool(false));

        self.call(
            "Emulation.setDeviceMetricsOverride",
            JsonValue::Object(params),
        )?;
        Ok(())
    }

    pub fn evaluate(&mut self, expression: &str) -> Result<JsonValue, String> {
        let resp = self.raw_evaluate(expression, true)?;
        Ok(resp
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(JsonValue::Null))
    }

    /// Evaluate and keep the result as a live JS handle, so actions operate on
    /// the element itself rather than re-running a selector for every step.
    fn evaluate_handle(&mut self, expression: &str) -> Result<Option<String>, String> {
        let resp = self.raw_evaluate(expression, false)?;
        Ok(resp
            .get("result")
            .and_then(|r| r.get("objectId"))
            .and_then(|o| o.as_str())
            .map(|s| s.to_string()))
    }

    fn raw_evaluate(&mut self, expression: &str, by_value: bool) -> Result<JsonValue, String> {
        let mut params = HashMap::new();
        params.insert(
            "expression".to_string(),
            JsonValue::String(expression.to_string()),
        );
        params.insert("returnByValue".to_string(), JsonValue::Bool(by_value));
        params.insert("awaitPromise".to_string(), JsonValue::Bool(true));

        if let Some(context_id) = self.target_context_id()? {
            params.insert(
                "contextId".to_string(),
                JsonValue::Number(context_id as f64),
            );
        }

        let resp = self.call("Runtime.evaluate", JsonValue::Object(params))?;
        throw_if_exception(&resp)?;
        Ok(resp)
    }

    /// Call `function_declaration` with the element handle as `this`.
    fn call_on(
        &mut self,
        object_id: &str,
        function_declaration: &str,
    ) -> Result<JsonValue, String> {
        let mut params = HashMap::new();
        params.insert(
            "objectId".to_string(),
            JsonValue::String(object_id.to_string()),
        );
        params.insert(
            "functionDeclaration".to_string(),
            JsonValue::String(function_declaration.to_string()),
        );
        params.insert("returnByValue".to_string(), JsonValue::Bool(true));
        params.insert("awaitPromise".to_string(), JsonValue::Bool(true));

        let resp = self.call("Runtime.callFunctionOn", JsonValue::Object(params))?;
        throw_if_exception(&resp)?;
        Ok(resp
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(JsonValue::Null))
    }

    fn release(&mut self, object_id: &str) {
        let mut params = HashMap::new();
        params.insert(
            "objectId".to_string(),
            JsonValue::String(object_id.to_string()),
        );
        let _ = self.call("Runtime.releaseObject", JsonValue::Object(params));
    }

    fn target_context_id(&mut self) -> Result<Option<i64>, String> {
        let Some(frame_id) = self.target_frame.clone() else {
            return Ok(None);
        };

        self.contexts
            .iter()
            .find(|c| c.frame_id == frame_id)
            .map(|c| Some(c.id))
            .ok_or_else(|| {
                "the targeted frame has no live execution context (it may have navigated away)"
                    .to_string()
            })
    }

    /// Resolve a locator to a live element handle, retrying until it is present
    /// and visible or `timeout_ms` runs out.
    ///
    /// Without this an action fails the instant it runs a step early, which is
    /// the common case on any app that renders asynchronously — the caller
    /// would otherwise have to hand-write a `wait-for` before every click.
    fn resolve(&mut self, spec: &str, timeout_ms: u64) -> Result<String, String> {
        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        loop {
            let reason;
            let candidate = match spec.strip_prefix("ref=") {
                Some(reference) => self.resolve_reference(reference),
                None => self.evaluate_handle(&locator::with_prelude(&format!(
                    "window.__wd.find(\"{}\")",
                    json::escape_str(spec)
                ))),
            };

            match candidate {
                Ok(Some(object_id)) => {
                    let visible = self.call_on(
                        &object_id,
                        r#"function() {
                            if (window.__wd) return window.__wd.visible(this);
                            if (!this || !this.getClientRects().length) return false;
                            const s = getComputedStyle(this);
                            return s.visibility !== 'hidden' && s.display !== 'none' && parseFloat(s.opacity) > 0;
                        }"#,
                    );
                    match visible {
                        Ok(JsonValue::Bool(true)) => return Ok(object_id),
                        _ => {
                            self.release(&object_id);
                            reason = format!("'{spec}' matched but is not visible");
                        }
                    }
                }
                Ok(None) => reason = format!("no element matched '{spec}'"),
                Err(e) => reason = e,
            }

            if start.elapsed() >= timeout {
                return Err(format!("{reason} after {timeout_ms}ms"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn resolve_reference(&mut self, reference: &str) -> Result<Option<String>, String> {
        let backend_node_id = *self.refs.get(reference).ok_or_else(|| {
            format!("unknown reference '{reference}' — run `snapshot` to refresh references")
        })?;

        let mut params = HashMap::new();
        params.insert(
            "backendNodeId".to_string(),
            JsonValue::Number(backend_node_id as f64),
        );
        if let Some(context_id) = self.target_context_id()? {
            params.insert(
                "executionContextId".to_string(),
                JsonValue::Number(context_id as f64),
            );
        }

        let resp = self
            .call("DOM.resolveNode", JsonValue::Object(params))
            .map_err(|_| {
                format!("reference '{reference}' is stale — run `snapshot` to refresh references")
            })?;

        Ok(resp
            .get("object")
            .and_then(|o| o.get("objectId"))
            .and_then(|o| o.as_str())
            .map(|s| s.to_string()))
    }

    /// Scroll into view and return the element's centre in *main frame*
    /// coordinates.
    ///
    /// `Input.dispatchMouseEvent` is always main-frame relative, while
    /// `getBoundingClientRect()` is relative to the element's own frame — so
    /// inside an iframe the two disagree by the iframe's offset and clicks land
    /// somewhere else entirely. `DOM.getContentQuads` reports in the space the
    /// input events actually use.
    fn centre_of(&mut self, object_id: &str) -> Result<(f64, f64), String> {
        self.call_on(
            object_id,
            r#"function() { this.scrollIntoView({ block: "center", inline: "center" }); }"#,
        )?;

        let mut params = HashMap::new();
        params.insert(
            "objectId".to_string(),
            JsonValue::String(object_id.to_string()),
        );
        let resp = self.call("DOM.getContentQuads", JsonValue::Object(params))?;

        let quad = resp
            .get("quads")
            .and_then(|q| q.as_array())
            .and_then(|quads| quads.first())
            .and_then(|q| q.as_array())
            .filter(|points| points.len() >= 8)
            .ok_or_else(|| "element has no layout box to click".to_string())?;

        let coordinate = |i: usize| quad[i].as_f64().unwrap_or(0.0);
        let x = (coordinate(0) + coordinate(2) + coordinate(4) + coordinate(6)) / 4.0;
        let y = (coordinate(1) + coordinate(3) + coordinate(5) + coordinate(7)) / 4.0;

        Ok((x, y))
    }

    fn mouse_event(&mut self, kind: &str, x: f64, y: f64, clicks: f64) -> Result<(), String> {
        let mut params = HashMap::new();
        params.insert("type".to_string(), JsonValue::String(kind.to_string()));
        params.insert("x".to_string(), JsonValue::Number(x));
        params.insert("y".to_string(), JsonValue::Number(y));
        if clicks > 0.0 {
            params.insert("button".to_string(), JsonValue::String("left".to_string()));
            params.insert("clickCount".to_string(), JsonValue::Number(clicks));
            params.insert("buttons".to_string(), JsonValue::Number(1.0));
        }
        self.call("Input.dispatchMouseEvent", JsonValue::Object(params))?;
        Ok(())
    }

    /// Wait until `spec` resolves to an element that is visible *and* painted.
    /// Two animation frames after the layout box appears rule out an `ok` that
    /// really means "matched unpainted SSR markup".
    pub fn wait_for(&mut self, spec: &str, timeout_ms: u64) -> Result<(), String> {
        let object_id = self.resolve(spec, timeout_ms)?;
        self.release(&object_id);
        self.evaluate(
            "new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(true))))",
        )?;
        Ok(())
    }

    /// Wait until the current URL contains `substring`.
    pub fn wait_for_url(&mut self, substring: &str, timeout_ms: u64) -> Result<String, String> {
        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            if let Ok(url) = self.current_url() {
                if url.contains(substring) {
                    return Ok(url);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        Err(format!(
            "timed out waiting for URL containing '{substring}' after {timeout_ms}ms"
        ))
    }

    /// Wait until the DOM stops mutating for `quiet_ms`. Generic "SPA settled"
    /// signal, preferable to guessing with a fixed sleep.
    pub fn wait_for_quiet(&mut self, quiet_ms: u64, timeout_ms: u64) -> Result<(), String> {
        let script = format!(
            r#"new Promise((resolve) => {{
                let last = Date.now();
                const deadline = Date.now() + {timeout_ms};
                const obs = new MutationObserver(() => {{ last = Date.now(); }});
                obs.observe(document.documentElement, {{
                    subtree: true, childList: true, attributes: true, characterData: true
                }});
                const iv = setInterval(() => {{
                    const settled = Date.now() - last >= {quiet_ms};
                    if (settled || Date.now() > deadline) {{
                        clearInterval(iv);
                        obs.disconnect();
                        resolve(settled);
                    }}
                }}, 50);
            }})"#
        );

        match self.evaluate(&script)? {
            JsonValue::Bool(true) => Ok(()),
            _ => Err(format!(
                "DOM never stayed quiet for {quiet_ms}ms within {timeout_ms}ms"
            )),
        }
    }

    pub fn reload(&mut self, timeout_ms: u64) -> Result<String, String> {
        self.call("Page.reload", JsonValue::Object(HashMap::new()))?;
        self.await_ready(timeout_ms);
        self.current_url()
    }

    pub fn current_url(&mut self) -> Result<String, String> {
        Ok(self
            .evaluate("location.href")?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub fn title(&mut self) -> Result<String, String> {
        Ok(self
            .evaluate("document.title")?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    /// Send a key chord such as `Enter`, `Escape` or `Meta+O` to the focused element.
    pub fn press(&mut self, chord: &str) -> Result<(), String> {
        let mut modifiers = 0u8;
        let mut key_part = chord;

        for part in chord.split('+') {
            match part.to_ascii_lowercase().as_str() {
                "alt" | "option" => modifiers |= 1,
                "ctrl" | "control" => modifiers |= 2,
                "meta" | "cmd" | "command" => modifiers |= 4,
                "shift" => modifiers |= 8,
                _ => key_part = part,
            }
        }

        let (key, code, vk, text) = key_descriptor(key_part)?;

        for event in ["keyDown", "keyUp"] {
            let mut params = HashMap::new();
            params.insert("type".to_string(), JsonValue::String(event.to_string()));
            params.insert("key".to_string(), JsonValue::String(key.clone()));
            params.insert("code".to_string(), JsonValue::String(code.clone()));
            params.insert("windowsVirtualKeyCode".to_string(), JsonValue::Number(vk));
            params.insert("nativeVirtualKeyCode".to_string(), JsonValue::Number(vk));
            params.insert(
                "modifiers".to_string(),
                JsonValue::Number(f64::from(modifiers)),
            );
            if event == "keyDown" && (modifiers & (1 | 2 | 4) == 0) {
                if let Some(ref t) = text {
                    params.insert("text".to_string(), JsonValue::String(t.clone()));
                }
            }
            self.call("Input.dispatchKeyEvent", JsonValue::Object(params))?;
        }

        Ok(())
    }

    /// Click via real mouse input, so `event.isTrusted` holds and libraries
    /// that listen for pointer events behave as they do for a person.
    pub fn click(&mut self, spec: &str, timeout_ms: u64) -> Result<(), String> {
        let object_id = self.resolve(spec, timeout_ms)?;
        let result = (|| -> Result<(), String> {
            let (x, y) = self.centre_of(&object_id)?;
            self.mouse_event("mouseMoved", x, y, 0.0)?;
            self.mouse_event("mousePressed", x, y, 1.0)?;
            self.mouse_event("mouseReleased", x, y, 1.0)
        })();
        self.release(&object_id);
        result
    }

    pub fn hover(&mut self, spec: &str, timeout_ms: u64) -> Result<(), String> {
        let object_id = self.resolve(spec, timeout_ms)?;
        let result = (|| -> Result<(), String> {
            let (x, y) = self.centre_of(&object_id)?;
            self.mouse_event("mouseMoved", x, y, 0.0)
        })();
        self.release(&object_id);
        result
    }

    /// Set a form field's value using real editing input.
    ///
    /// `Input.insertText` goes through the browser's own text pipeline, so a
    /// React controlled input sees the same events it would from a person —
    /// assigning `el.value` is swallowed by React's value tracker.
    pub fn type_text(
        &mut self,
        spec: &str,
        text: &str,
        clear: bool,
        timeout_ms: u64,
    ) -> Result<(), String> {
        let object_id = self.resolve(spec, timeout_ms)?;

        let result = (|| -> Result<(), String> {
            let (x, y) = self.centre_of(&object_id)?;
            self.mouse_event("mouseMoved", x, y, 0.0)?;
            self.mouse_event("mousePressed", x, y, 1.0)?;
            self.mouse_event("mouseReleased", x, y, 1.0)?;

            self.call_on(
                &object_id,
                if clear {
                    r#"function() {
                        this.focus();
                        if (typeof this.select === "function") { this.select(); }
                        else if (this.isContentEditable) { document.execCommand("selectAll"); }
                    }"#
                } else {
                    r#"function() {
                        this.focus();
                        const end = (this.value || "").length;
                        if (typeof this.setSelectionRange === "function") {
                            try { this.setSelectionRange(end, end); } catch (_) {}
                        }
                    }"#
                },
            )?;

            if clear && text.is_empty() {
                self.press("Backspace")?;
                return Ok(());
            }

            let mut params = HashMap::new();
            params.insert("text".to_string(), JsonValue::String(text.to_string()));
            self.call("Input.insertText", JsonValue::Object(params))?;
            Ok(())
        })();

        self.release(&object_id);
        result
    }

    pub fn capture_screenshot(
        &mut self,
        full_page: bool,
        selector: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let mut params = HashMap::new();
        params.insert("format".to_string(), JsonValue::String("png".to_string()));

        if let Some(sel) = selector {
            let object_id = self.resolve(sel, self.default_timeout_ms)?;
            let clip_val = self.call_on(
                &object_id,
                r#"function() {
                    this.scrollIntoView({ block: "center", inline: "center" });
                    const r = this.getBoundingClientRect();
                    return { x: r.left + window.scrollX, y: r.top + window.scrollY, width: r.width, height: r.height };
                }"#,
            );
            self.release(&object_id);
            let clip_val = clip_val?;

            let x = clip_val.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = clip_val.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let w = clip_val
                .get("width")
                .and_then(|v| v.as_f64())
                .unwrap_or(100.0);
            let h = clip_val
                .get("height")
                .and_then(|v| v.as_f64())
                .unwrap_or(100.0);

            let mut clip = HashMap::new();
            clip.insert("x".to_string(), JsonValue::Number(x));
            clip.insert("y".to_string(), JsonValue::Number(y));
            clip.insert("width".to_string(), JsonValue::Number(w));
            clip.insert("height".to_string(), JsonValue::Number(h));
            clip.insert("scale".to_string(), JsonValue::Number(1.0));

            params.insert("clip".to_string(), JsonValue::Object(clip));
        } else if full_page {
            params.insert("captureBeyondViewport".to_string(), JsonValue::Bool(true));
        }

        let resp = self.call("Page.captureScreenshot", JsonValue::Object(params))?;
        let data_b64 = resp
            .get("data")
            .and_then(|d| d.as_str())
            .ok_or_else(|| "missing 'data' in screenshot response".to_string())?;

        base64::decode(data_b64).map_err(|e| format!("failed to decode screenshot base64: {e}"))
    }

    pub fn print_to_pdf(&mut self) -> Result<Vec<u8>, String> {
        let mut params = HashMap::new();
        params.insert("printBackground".to_string(), JsonValue::Bool(true));

        let resp = self.call("Page.printToPDF", JsonValue::Object(params))?;
        let data_b64 = resp
            .get("data")
            .and_then(|d| d.as_str())
            .ok_or_else(|| "missing 'data' in PDF response".to_string())?;

        base64::decode(data_b64).map_err(|e| format!("failed to decode PDF base64: {e}"))
    }

    pub fn get_html(&mut self) -> Result<String, String> {
        let val = self.evaluate("document.documentElement.outerHTML")?;
        val.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "failed to retrieve HTML string".to_string())
    }

    /// Snapshot the accessibility tree as `eN role "name"` lines.
    ///
    /// This is what lets an agent act without inventing selectors: every line
    /// carries a `ref=eN` handle that `click`/`fill`/`hover` accept, resolved
    /// through the DOM node the a11y tree points at. Chrome computes the names,
    /// so `aria-labelledby` and `<label for>` are honoured properly.
    pub fn snapshot(&mut self, all: bool, limit: usize) -> Result<Vec<SnapshotNode>, String> {
        let mut params = HashMap::new();
        // Scope the tree to the targeted frame, otherwise `frame` selects an
        // iframe and `snapshot` keeps describing the page around it.
        if let Some(ref frame_id) = self.target_frame {
            params.insert("frameId".to_string(), JsonValue::String(frame_id.clone()));
        }
        let resp = self.call("Accessibility.getFullAXTree", JsonValue::Object(params))?;
        let nodes: Vec<JsonValue> = resp
            .get("nodes")
            .and_then(|n| n.as_array())
            .map(<[JsonValue]>::to_vec)
            .unwrap_or_default();

        self.refs.clear();
        let mut out = Vec::new();

        for node in nodes {
            if node.get("ignored").and_then(|i| i.as_bool()) == Some(true) {
                continue;
            }
            let Some(backend_node_id) = node.get("backendDOMNodeId").and_then(|b| b.as_i64())
            else {
                continue;
            };
            let role = node
                .get("role")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = node
                .get("name")
                .and_then(|n| n.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if !all && !is_interesting_role(&role) {
                continue;
            }
            if role.is_empty() || (name.is_empty() && !all) {
                continue;
            }

            let reference = format!("e{}", out.len() + 1);
            self.refs.insert(reference.clone(), backend_node_id);
            out.push(SnapshotNode {
                reference,
                role,
                name,
                backend_node_id,
            });

            if out.len() >= limit {
                break;
            }
        }

        Ok(out)
    }

    /// Aim subsequent commands at a frame whose URL or name contains
    /// `substring`. Without this everything runs against the top frame, which
    /// makes an OAuth or payment iframe invisible to the driver.
    pub fn target_frame(&mut self, substring: Option<&str>) -> Result<String, String> {
        let Some(substring) = substring else {
            self.target_frame = None;
            return Ok("top".to_string());
        };

        let frames = self.frame_tree()?;
        let hit = frames
            .iter()
            .find(|(_, url, name)| url.contains(substring) || name.contains(substring))
            .ok_or_else(|| {
                let known: Vec<String> = frames
                    .iter()
                    .map(|(_, url, _)| truncate_middle(url, 60))
                    .collect();
                format!(
                    "no frame matching '{substring}' (frames: {})",
                    known.join(", ")
                )
            })?;

        self.target_frame = Some(hit.0.clone());
        // Surface the missing context now rather than on the next command.
        self.target_context_id()?;
        Ok(truncate_middle(&hit.1, 80))
    }

    fn frame_tree(&mut self) -> Result<Vec<(String, String, String)>, String> {
        let resp = self.call("Page.getFrameTree", JsonValue::Object(HashMap::new()))?;
        let mut out = Vec::new();
        if let Some(tree) = resp.get("frameTree") {
            collect_frames(tree, &mut out);
        }
        Ok(out)
    }

    pub fn close_browser(&mut self) -> Result<(), String> {
        let _ = self.call("Browser.close", JsonValue::Object(HashMap::new()));
        Ok(())
    }
}

/// Best-effort human rendering of a CDP `RemoteObject` for the console buffer.
fn describe_remote_object(obj: &JsonValue) -> String {
    if let Some(desc) = obj.get("description").and_then(|d| d.as_str()) {
        return desc.to_string();
    }
    match obj.get("value") {
        Some(JsonValue::String(s)) => s.clone(),
        Some(other) => other.to_json_string(),
        None => obj
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

/// Map a key name to the `(key, code, virtualKeyCode, text)` CDP needs.
fn key_descriptor(name: &str) -> Result<(String, String, f64, Option<String>), String> {
    let named = match name {
        "Enter" | "enter" | "Return" => Some(("Enter", "Enter", 13.0, Some("\r"))),
        "Tab" | "tab" => Some(("Tab", "Tab", 9.0, Some("\t"))),
        "Escape" | "escape" | "Esc" | "esc" => Some(("Escape", "Escape", 27.0, None)),
        "Backspace" | "backspace" => Some(("Backspace", "Backspace", 8.0, None)),
        "Delete" | "delete" => Some(("Delete", "Delete", 46.0, None)),
        "Space" | "space" => Some((" ", "Space", 32.0, Some(" "))),
        "ArrowUp" | "Up" => Some(("ArrowUp", "ArrowUp", 38.0, None)),
        "ArrowDown" | "Down" => Some(("ArrowDown", "ArrowDown", 40.0, None)),
        "ArrowLeft" | "Left" => Some(("ArrowLeft", "ArrowLeft", 37.0, None)),
        "ArrowRight" | "Right" => Some(("ArrowRight", "ArrowRight", 39.0, None)),
        "Home" | "home" => Some(("Home", "Home", 36.0, None)),
        "End" | "end" => Some(("End", "End", 35.0, None)),
        "PageUp" => Some(("PageUp", "PageUp", 33.0, None)),
        "PageDown" => Some(("PageDown", "PageDown", 34.0, None)),
        _ => None,
    };

    if let Some((key, code, vk, text)) = named {
        return Ok((
            key.to_string(),
            code.to_string(),
            vk,
            text.map(|t| t.to_string()),
        ));
    }

    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => {
            let upper = ch.to_ascii_uppercase();
            let code = if ch.is_ascii_alphabetic() {
                format!("Key{upper}")
            } else if ch.is_ascii_digit() {
                format!("Digit{ch}")
            } else {
                String::new()
            };
            Ok((
                ch.to_string(),
                code,
                upper as u32 as f64,
                Some(ch.to_string()),
            ))
        }
        _ => Err(format!("unknown key '{name}'")),
    }
}

fn throw_if_exception(resp: &JsonValue) -> Result<(), String> {
    if let Some(ex) = resp.get("exceptionDetails") {
        let desc = ex
            .get("exception")
            .and_then(|e| e.get("description"))
            .and_then(|d| d.as_str())
            .or_else(|| ex.get("text").and_then(|t| t.as_str()))
            .unwrap_or("JS evaluation threw an exception");
        return Err(format!("JS error: {desc}"));
    }
    Ok(())
}

/// Roles worth listing by default: the things an agent can act on, plus the
/// landmarks it needs to know where it is.
fn is_interesting_role(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "link"
            | "textbox"
            | "searchbox"
            | "checkbox"
            | "radio"
            | "combobox"
            | "listbox"
            | "option"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "tab"
            | "switch"
            | "slider"
            | "spinbutton"
            | "heading"
            | "dialog"
            | "alert"
            | "alertdialog"
            | "status"
            | "img"
    )
}

fn collect_frames(tree: &JsonValue, out: &mut Vec<(String, String, String)>) {
    if let Some(frame) = tree.get("frame") {
        let id = frame
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string();
        let url = frame
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        let name = frame
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        if !id.is_empty() {
            out.push((id, url, name));
        }
    }

    if let Some(children) = tree.get("childFrames").and_then(|c| c.as_array()) {
        for child in children {
            collect_frames(child, out);
        }
    }
}

fn truncate_middle(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let head: String = s.chars().take(limit / 2).collect();
    let tail: String = s
        .chars()
        .skip(s.chars().count().saturating_sub(limit / 2))
        .collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn handshake_rejects_oversized_headers_without_crlf_terminator() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = [0u8; 256];
            let _ = sock.read(&mut req);
            // Send 101 status line followed by >8192 bytes without \r\n\r\n
            let mut resp = b"HTTP/1.1 101 Switching Protocols\r\nX-Pad: ".to_vec();
            resp.resize(9000, b'a');
            let _ = sock.write_all(&resp);
        });

        let res = WebSocketClient::connect("127.0.0.1", port, "/devtools/page/1");
        assert!(res.is_err());
        let err = res.err().unwrap();
        assert!(
            err.contains("exceeded 8192 bytes"),
            "unexpected error: {err}"
        );
        server.join().unwrap();
    }

    #[test]
    fn read_text_replies_to_ping_with_capped_pong_and_returns_text_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = [0u8; 512];
            let _ = sock.read(&mut req);
            sock.write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
            )
            .unwrap();

            // Send a Ping frame with 130 bytes (16-bit length = 126)
            let ping_payload = vec![b'p'; 130];
            let mut ping_frame = vec![0x89, 126];
            ping_frame.extend_from_slice(&(130u16).to_be_bytes());
            ping_frame.extend_from_slice(&ping_payload);
            sock.write_all(&ping_frame).unwrap();

            // Read the client's Pong frame and verify header length matches payload bytes (125)
            let mut pong_hdr = [0u8; 2];
            sock.read_exact(&mut pong_hdr).unwrap();
            assert_eq!(pong_hdr[0], 0x8A);
            assert_eq!(pong_hdr[1] & 0x80, 0x80);
            let pong_len = (pong_hdr[1] & 0x7f) as usize;
            assert_eq!(pong_len, 125);
            let mut mask_and_body = vec![0u8; 4 + pong_len];
            sock.read_exact(&mut mask_and_body).unwrap();
            assert_eq!(&mask_and_body[4..], &vec![b'p'; 125][..]);

            // Now send a text frame "hello"
            let text_frame = [0x81, 5, b'h', b'e', b'l', b'l', b'o'];
            sock.write_all(&text_frame).unwrap();
        });

        let mut client = WebSocketClient::connect("127.0.0.1", port, "/devtools/page/1").unwrap();
        let msg = client.read_text().unwrap();
        assert_eq!(msg, "hello");
        server.join().unwrap();
    }

    #[test]
    fn read_text_rejects_oversized_64bit_frame_before_allocating() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = [0u8; 512];
            let _ = sock.read(&mut req);
            sock.write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
            )
            .unwrap();

            // Send a header claiming 512 MiB payload
            let mut frame = vec![0x81, 127];
            frame.extend_from_slice(&((512u64 * 1024 * 1024).to_be_bytes()));
            sock.write_all(&frame).unwrap();
        });

        let mut client = WebSocketClient::connect("127.0.0.1", port, "/devtools/page/1").unwrap();
        let err = client.read_text().unwrap_err();
        assert!(err.contains("exceeds maximum allowed"), "got: {err}");
        server.join().unwrap();
    }
}
