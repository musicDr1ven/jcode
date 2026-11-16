//! Plugin child-process management: spawn, framing, handshake, and teardown.
//!
//! The transport is behind a small trait so protocol tests can drive a scripted
//! peer without spawning a process. The real transport owns one child process
//! per configured plugin per session and is killed on teardown.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use super::protocol;
use crate::config::{PluginCapability, PluginConfig};

/// One line-oriented conversation with a plugin. Implementations must be safe to
/// call from several tasks; ordering is owned by the caller's request lock.
#[async_trait::async_trait]
pub(crate) trait PluginTransport: Send + Sync {
    async fn write_line(&self, line: &str) -> Result<()>;
    async fn read_line(&self) -> Result<Option<String>>;
    /// Close the plugin's stdin, giving it EOF so it can flush and exit.
    async fn close_input(&self) {}
    /// Wait (bounded) for the plugin to exit on its own. `true` when it did.
    async fn wait_for_exit(&self, _timeout: Duration) -> bool {
        false
    }
    async fn kill(&self);
}

/// A spawned plugin, ready to serve its declared extension points.
pub(crate) struct PluginProcess {
    plugin_name: String,
    reported_name: String,
    version: String,
    protocol_version: u32,
    declared: Vec<PluginCapability>,
    granted: Vec<PluginCapability>,
    transport: Box<dyn PluginTransport>,
    request_lock: Mutex<()>,
    next_id: AtomicU64,
}

impl PluginProcess {
    pub(crate) fn reported_name(&self) -> &str {
        &self.reported_name
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    pub(crate) fn protocol_version(&self) -> u32 {
        self.protocol_version
    }

    pub(crate) fn declared(&self) -> &[PluginCapability] {
        &self.declared
    }

    /// The capabilities this plugin both declared and configuration granted.
    pub(crate) fn granted(&self) -> &[PluginCapability] {
        &self.granted
    }

    pub(crate) fn supports(&self, capability: PluginCapability) -> bool {
        self.granted.contains(&capability)
    }

    pub(crate) async fn notify(
        &self,
        event: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<()> {
        let (payload, truncated) = protocol::cap_payload(event, payload);
        if truncated {
            crate::logging::warn(&format!(
                "Plugin '{}' event '{event}' payload exceeded {} bytes; sent a capped envelope",
                self.plugin_name,
                protocol::EVENT_PAYLOAD_MAX_BYTES
            ));
        }
        let line = protocol::event(event, payload).to_string();
        let _guard = self.request_lock.lock().await;
        tokio::time::timeout(timeout, self.transport.write_line(&line))
            .await
            .map_err(|_| {
                anyhow!(
                    "plugin '{}' did not accept event '{event}' within {}ms",
                    self.plugin_name,
                    timeout.as_millis()
                )
            })?
            .with_context(|| format!("writing event '{event}' to plugin '{}'", self.plugin_name))
    }

    pub(crate) async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let _guard = self.request_lock.lock().await;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let line = protocol::request(id, method, params).to_string();
        self.transport
            .write_line(&line)
            .await
            .with_context(|| format!("writing {method} to plugin '{}'", self.plugin_name))?;

        let deadline = Instant::now() + timeout;
        let mut malformed = 0usize;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!(
                    "plugin '{}' did not answer {method} within {}ms",
                    self.plugin_name,
                    timeout.as_millis()
                );
            }
            let Some(line) = tokio::time::timeout(remaining, self.transport.read_line())
                .await
                .map_err(|_| {
                    anyhow!(
                        "plugin '{}' did not answer {method} within {}ms",
                        self.plugin_name,
                        timeout.as_millis()
                    )
                })??
            else {
                bail!(
                    "plugin '{}' closed its output while answering {method}",
                    self.plugin_name
                );
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                malformed += 1;
                crate::logging::warn(&format!(
                    "Plugin '{}' wrote a non-protocol line ({} bytes); discarding it",
                    self.plugin_name,
                    trimmed.len()
                ));
                if malformed > 32 {
                    bail!(
                        "plugin '{}' produced too many malformed lines",
                        self.plugin_name
                    );
                }
                continue;
            };
            match value.get("type").and_then(Value::as_str) {
                Some("response") if value.get("id").and_then(Value::as_u64) == Some(id) => {
                    if value.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                    }
                    bail!(
                        "plugin '{}' rejected {method}: {}",
                        self.plugin_name,
                        value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("no reason given")
                    );
                }
                Some("response") => {
                    crate::logging::warn(&format!(
                        "Plugin '{}' answered an unknown request id while {method} was in flight",
                        self.plugin_name
                    ));
                }
                other => {
                    crate::logging::warn(&format!(
                        "Plugin '{}' sent unsolicited message type {:?} during {method}",
                        self.plugin_name, other
                    ));
                }
            }
        }
    }

    /// Close the plugin's stdin so it sees EOF and can flush its state.
    pub(crate) async fn close_input(&self) {
        self.transport.close_input().await;
    }

    /// Wait, bounded, for the plugin to exit after EOF. `true` when it exited.
    pub(crate) async fn wait_for_exit(&self, timeout: Duration) -> bool {
        self.transport.wait_for_exit(timeout).await
    }

    pub(crate) async fn kill(&self) {
        self.transport.kill().await;
    }
}

struct ChildTransport {
    plugin_name: String,
    child: Mutex<tokio::process::Child>,
    stdin: Mutex<Option<tokio::process::ChildStdin>>,
    stdout: Mutex<BufReader<tokio::process::ChildStdout>>,
}

#[async_trait::async_trait]
impl PluginTransport for ChildTransport {
    async fn write_line(&self, line: &str) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        let Some(stdin) = stdin.as_mut() else {
            bail!("plugin '{}' stdin is closed", self.plugin_name);
        };
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_line(&self) -> Result<Option<String>> {
        let mut stdout = self.stdout.lock().await;
        let mut line = String::new();
        let read = stdout.read_line(&mut line).await?;
        if read == 0 {
            return Ok(None);
        }
        Ok(Some(line))
    }

    async fn close_input(&self) {
        let mut stdin = self.stdin.lock().await;
        *stdin = None;
    }

    async fn wait_for_exit(&self, timeout: Duration) -> bool {
        let mut child = self.child.lock().await;
        matches!(tokio::time::timeout(timeout, child.wait()).await, Ok(Ok(_)))
    }

    async fn kill(&self) {
        let mut stdin = self.stdin.lock().await;
        *stdin = None;
        drop(stdin);
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

/// Spawn a configured plugin and complete the versioned handshake.
///
/// Returns an error when the command cannot start, the plugin never answers, or
/// it reports a protocol version this build does not speak. The error is the
/// diagnostic shown to the operator, so it names the plugin and both versions.
pub(crate) async fn spawn_plugin(
    config: &PluginConfig,
    session_id: &str,
    cwd: Option<&str>,
) -> Result<PluginProcess> {
    validate_plugin_config(config)?;
    let mut command = tokio::process::Command::new(&config.command);
    command
        .args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("JCODE_PLUGIN_NAME", &config.name)
        .env("JCODE_PLUGIN_PROTOCOL", protocol::PROTOCOL_VERSION.to_string());
    let mut child = command.spawn().with_context(|| {
        format!(
            "failed to start plugin '{}' ({} {:?})",
            config.name, config.command, config.args
        )
    })?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("plugin '{}' has no stdin pipe", config.name))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("plugin '{}' has no stdout pipe", config.name))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("plugin '{}' has no stderr pipe", config.name))?;

    let stderr_name = config.name.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let trimmed = line.trim_end();
                    if !trimmed.is_empty() {
                        let capped: String = trimmed.chars().take(2000).collect();
                        crate::logging::warn(&format!(
                            "Plugin '{stderr_name}' stderr: {capped}"
                        ));
                    }
                }
            }
        }
    });

    let transport = ChildTransport {
        plugin_name: config.name.clone(),
        child: Mutex::new(child),
        stdin: Mutex::new(Some(stdin)),
        stdout: Mutex::new(BufReader::new(stdout)),
    };
    handshake(config, session_id, cwd, Box::new(transport), config.timeouts.handshake_ms).await
}

/// Complete the handshake over an already connected transport.
pub(crate) async fn handshake(
    config: &PluginConfig,
    session_id: &str,
    cwd: Option<&str>,
    transport: Box<dyn PluginTransport>,
    handshake_timeout_ms: u64,
) -> Result<PluginProcess> {
    let process = PluginProcess {
        plugin_name: config.name.clone(),
        reported_name: config.name.clone(),
        version: "unknown".to_string(),
        protocol_version: protocol::PROTOCOL_VERSION,
        declared: Vec::new(),
        granted: config.capabilities.clone(),
        transport,
        request_lock: Mutex::new(()),
        next_id: AtomicU64::new(2),
    };

    let granted: Vec<&str> = config
        .capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect();
    let hello = protocol::hello(1, session_id, cwd, &granted);
    let deadline = Duration::from_millis(handshake_timeout_ms.max(1));
    process
        .transport
        .write_line(&hello.to_string())
        .await
        .with_context(|| format!("sending hello to plugin '{}'", config.name))?;

    let started = Instant::now();
    let mut malformed = 0usize;
    loop {
        let remaining = deadline
            .checked_sub(started.elapsed())
            .unwrap_or_default();
        if remaining.is_zero() {
            bail!(
                "plugin '{}' did not complete the handshake within {}ms",
                config.name,
                handshake_timeout_ms
            );
        }
        let Some(line) = tokio::time::timeout(remaining, process.transport.read_line())
            .await
            .map_err(|_| {
                anyhow!(
                    "plugin '{}' did not complete the handshake within {}ms",
                    config.name,
                    handshake_timeout_ms
                )
            })??
        else {
            bail!(
                "plugin '{}' exited during the handshake",
                config.name
            );
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            malformed += 1;
            crate::logging::warn(&format!(
                "Plugin '{}' wrote a non-protocol line during the handshake; discarding it",
                config.name
            ));
            if malformed > 32 {
                bail!("plugin '{}' never produced a valid handshake", config.name);
            }
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("hello") => {
                protocol::check_hello_protocol(&value).map_err(anyhow::Error::msg)?;
                let reported = value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&config.name)
                    .to_string();
                let version = value
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let declared = parse_declared_capabilities(&value, &config.name);
                let granted: Vec<PluginCapability> = config
                    .capabilities
                    .iter()
                    .copied()
                    .filter(|capability| declared.contains(capability))
                    .collect();
                for missing in config
                    .capabilities
                    .iter()
                    .filter(|capability| !declared.contains(capability))
                {
                    crate::logging::warn(&format!(
                        "Plugin '{}' was granted '{}' but did not declare it; it will not be used",
                        config.name,
                        missing.as_str()
                    ));
                }
                return Ok(PluginProcess {
                    plugin_name: config.name.clone(),
                    reported_name: reported,
                    version,
                    protocol_version: protocol::PROTOCOL_VERSION,
                    declared,
                    granted,
                    transport: process.transport,
                    request_lock: Mutex::new(()),
                    next_id: AtomicU64::new(2),
                });
            }
            Some(other) => {
                crate::logging::warn(&format!(
                    "Plugin '{}' sent {other:?} before the handshake completed; ignoring",
                    config.name
                ));
            }
            None => {
                malformed += 1;
                if malformed > 32 {
                    bail!("plugin '{}' never produced a valid handshake", config.name);
                }
            }
        }
    }
}

fn parse_declared_capabilities(value: &Value, plugin_name: &str) -> Vec<PluginCapability> {
    let Some(items) = value.get("capabilities").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut capabilities = Vec::new();
    for item in items {
        let Some(raw) = item.as_str() else {
            continue;
        };
        match PluginCapability::parse(raw) {
            Some(capability) if !capabilities.contains(&capability) => {
                capabilities.push(capability)
            }
            Some(_) => {}
            None => crate::logging::warn(&format!(
                "Plugin '{plugin_name}' declared unknown capability '{raw}'; ignoring it"
            )),
        }
    }
    capabilities
}

fn validate_plugin_config(config: &PluginConfig) -> Result<()> {
    if config.name.trim().is_empty() {
        bail!("a [[plugins]] entry has an empty name");
    }
    if config.name.contains(protocol::TOOL_NAME_SEPARATOR) {
        bail!(
            "plugin name '{}' may not contain '{}' (reserved for tool names)",
            config.name,
            protocol::TOOL_NAME_SEPARATOR
        );
    }
    if !config
        .name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!(
            "plugin name '{}' may only contain letters, digits, '-', '_', and '.'",
            config.name
        );
    }
    if config.command.trim().is_empty() {
        bail!("plugin '{}' has no command", config.name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    /// Scripted peer: pops queued inbound lines and records outbound ones.
    struct MockTransport {
        inbound: StdMutex<VecDeque<Option<String>>>,
        outbound: StdMutex<Vec<String>>,
        killed: StdMutex<bool>,
    }

    impl MockTransport {
        fn new(lines: Vec<&str>) -> Self {
            Self {
                inbound: StdMutex::new(lines.into_iter().map(|line| Some(line.to_string())).collect()),
                outbound: StdMutex::new(Vec::new()),
                killed: StdMutex::new(false),
            }
        }
    }

    #[async_trait::async_trait]
    impl PluginTransport for MockTransport {
        async fn write_line(&self, line: &str) -> Result<()> {
            self.outbound.lock().unwrap().push(line.to_string());
            Ok(())
        }

        async fn read_line(&self) -> Result<Option<String>> {
            Ok(self.inbound.lock().unwrap().pop_front().flatten())
        }

        async fn kill(&self) {
            *self.killed.lock().unwrap() = true;
        }
    }

    fn config(capabilities: Vec<PluginCapability>) -> PluginConfig {
        PluginConfig {
            name: "mock".to_string(),
            command: "mock".to_string(),
            capabilities,
            ..PluginConfig::default()
        }
    }

    fn hello_line() -> String {
        json!({
            "type": "hello",
            "protocol": protocol::PROTOCOL_VERSION,
            "name": "mock-plugin",
            "version": "1.2.3",
            "capabilities": ["observe", "context", "gate", "tools", "tool_events"],
        })
        .to_string()
    }

    #[tokio::test]
    async fn handshake_intersects_declared_capabilities_with_the_grant() {
        let transport = Box::new(MockTransport::new(vec![hello_line().as_str()]));
        let process = handshake(
            &config(vec![PluginCapability::Observe, PluginCapability::Context]),
            "ses_1",
            Some("/tmp"),
            transport,
            500,
        )
        .await
        .expect("handshake");
        assert_eq!(process.reported_name(), "mock-plugin");
        assert_eq!(process.version(), "1.2.3");
        assert!(process.supports(PluginCapability::Observe));
        assert!(process.supports(PluginCapability::Context));
        assert!(!process.supports(PluginCapability::Gate));
    }

    #[tokio::test]
    async fn handshake_refuses_an_unsupported_protocol_version_naming_both() {
        let bad = json!({
            "type": "hello", "protocol": 7, "name": "old", "version": "0.1", "capabilities": []
        })
        .to_string();
        let Err(error) = handshake(
            &config(vec![PluginCapability::Observe]),
            "ses_1",
            None,
            Box::new(MockTransport::new(vec![bad.as_str()])),
            500,
        )
        .await
        else {
            panic!("v7 must be refused");
        };
        let message = error.to_string();
        assert!(message.contains("v7") && message.contains("v1"), "{message}");
    }

    #[tokio::test]
    async fn handshake_times_out_on_a_silent_plugin() {
        let Err(error) = handshake(
            &config(vec![PluginCapability::Observe]),
            "ses_1",
            None,
            Box::new(MockTransport::new(Vec::new())),
            50,
        )
        .await
        else {
            panic!("silence must time out");
        };
        assert!(error.to_string().contains("handshake"), "{error}");
    }

    #[tokio::test]
    async fn request_correlates_by_id_and_skips_malformed_lines() {
        let response = json!({"type": "response", "id": 2, "ok": true, "result": {"text": "hi"}})
            .to_string();
        let transport = Box::new(MockTransport::new(vec![
            hello_line().as_str(),
            "not json",
            response.as_str(),
        ]));
        let process = handshake(
            &config(vec![PluginCapability::Context]),
            "ses_1",
            None,
            transport,
            500,
        )
        .await
        .expect("handshake");
        let result = process
            .request("context", json!({}), Duration::from_millis(500))
            .await
            .expect("context response");
        assert_eq!(result["text"], json!("hi"));
    }

    #[tokio::test]
    async fn request_reports_a_rejection_reason() {
        let response = json!({"type": "response", "id": 2, "ok": false, "error": "no tools"}).to_string();
        let transport = Box::new(MockTransport::new(vec![hello_line().as_str(), response.as_str()]));
        let process = handshake(
            &config(vec![PluginCapability::Tools]),
            "ses_1",
            None,
            transport,
            500,
        )
        .await
        .expect("handshake");
        let Err(error) = process
            .request("tools/list", json!({}), Duration::from_millis(500))
            .await
        else {
            panic!("rejection must surface");
        };
        assert!(error.to_string().contains("no tools"), "{error}");
    }
}
