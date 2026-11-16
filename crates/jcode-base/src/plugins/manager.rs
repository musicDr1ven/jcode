//! Session-scoped plugin registry and the four extension points.
//!
//! One process per configured plugin per session, started on first use and
//! killed at session end. With no `[[plugins]]` configured the public helpers
//! return immediately, so the default path gains no work beyond one cached
//! config read.

use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

use super::host::{self, PluginProcess};
use super::protocol;
use crate::config::{PluginCapability, PluginConfig, PluginGateFallback};

/// Context one plugin contributed for the current turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginContribution {
    pub plugin: String,
    pub text: String,
    pub truncated: bool,
}

/// The composed result of running every gate on one tool call.
#[derive(Debug, Clone)]
pub struct GateOutcome {
    pub input: Value,
    pub decision: GateDecision,
}

#[derive(Debug, Clone)]
pub enum GateDecision {
    Allow,
    Deny(String),
}

/// A tool offered by a plugin, already validated and prefixed for the model.
#[derive(Debug, Clone)]
pub struct PluginToolSpec {
    /// Model-facing name: `<plugin>__<name>`.
    pub flattened_name: String,
    pub plugin: String,
    /// The plugin's own name for the tool, used when calling it back.
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Outcome of `jcode plugin test` for one plugin.
#[derive(Debug, Clone)]
pub struct PluginProbeReport {
    pub name: String,
    pub usable: bool,
    pub reported_name: Option<String>,
    pub version: Option<String>,
    pub protocol: Option<u32>,
    pub declared: Vec<PluginCapability>,
    pub granted: Vec<PluginCapability>,
    pub error: Option<String>,
}

enum SlotState {
    Idle,
    Ready(Arc<PluginProcess>),
    Unusable,
}

struct Slot {
    config: PluginConfig,
    state: AsyncMutex<SlotState>,
}

struct SessionHost {
    session_id: String,
    cwd: Option<String>,
    slots: Vec<Slot>,
}

struct PluginRuntime {
    sessions: Mutex<HashMap<String, Arc<SessionHost>>>,
    /// Sessions that have ended. A late extension-point call must not revive a
    /// plugin process after teardown.
    ended: Mutex<HashSet<String>>,
}

static RUNTIME: OnceLock<PluginRuntime> = OnceLock::new();

fn runtime() -> &'static PluginRuntime {
    RUNTIME.get_or_init(|| PluginRuntime {
        sessions: Mutex::new(HashMap::new()),
        ended: Mutex::new(HashSet::new()),
    })
}

/// Whether any plugin is configured. Single cheap config read; the no-plugin
/// path is the ordinary path.
pub fn enabled() -> bool {
    !crate::config::config().plugins.is_empty()
}

fn configured_plugins() -> Vec<PluginConfig> {
    crate::config::config()
        .plugins
        .iter()
        .filter(|plugin| !plugin.name.trim().is_empty())
        .cloned()
        .collect()
}

fn session_host(session_id: &str, cwd: Option<&str>) -> Option<Arc<SessionHost>> {
    if runtime()
        .ended
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(session_id)
    {
        return None;
    }
    let mut sessions = runtime()
        .sessions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(host) = sessions.get(session_id) {
        return Some(Arc::clone(host));
    }
    let plugins = configured_plugins();
    if plugins.is_empty() {
        return None;
    }
    let host = Arc::new(SessionHost {
        session_id: session_id.to_string(),
        cwd: cwd.map(str::to_string),
        slots: plugins
            .into_iter()
            .map(|config| Slot {
                config,
                state: AsyncMutex::new(SlotState::Idle),
            })
            .collect(),
    });
    sessions.insert(session_id.to_string(), Arc::clone(&host));
    Some(host)
}

impl SessionHost {
    /// Start the plugin if needed and return it, or `None` once it is known
    /// unusable. A plugin is never restarted within a session.
    async fn ready(&self, slot: &Slot) -> Option<Arc<PluginProcess>> {
        let mut state = slot.state.lock().await;
        match &*state {
            SlotState::Ready(process) => Some(Arc::clone(process)),
            SlotState::Unusable => None,
            SlotState::Idle => {
                match host::spawn_plugin(
                    &slot.config,
                    &self.session_id,
                    self.cwd.as_deref(),
                )
                .await
                {
                    Ok(process) => {
                        let process = Arc::new(process);
                        *state = SlotState::Ready(Arc::clone(&process));
                        crate::logging::info(&format!(
                            "Plugin '{}' ready (v{}, protocol v{})",
                            slot.config.name,
                            process.version(),
                            process.protocol_version()
                        ));
                        Some(process)
                    }
                    Err(error) => {
                        let message = error.to_string();
                        crate::logging::warn(&format!(
                            "Plugin '{}' is unusable for this session: {message}",
                            slot.config.name
                        ));
                        *state = SlotState::Unusable;
                        None
                    }
                }
            }
        }
    }

    async fn mark_unusable(&self, slot: &Slot, reason: &str) {
        let mut state = slot.state.lock().await;
        if let SlotState::Ready(process) = &*state {
            crate::logging::warn(&format!(
                "Plugin '{}' failed and will not be restarted this session: {reason}",
                slot.config.name
            ));
            process.kill().await;
        }
        *state = SlotState::Unusable;
    }
}

/// Register the session (first use) and deliver `session_start` to plugins that
/// observe lifecycle events.
pub fn session_started(session_id: &str, cwd: Option<&str>, payload: Value) {
    emit_observe_event(session_id, cwd, "session_start", payload);
}

/// Deliver one lifecycle event to observe-capable plugins.
pub fn emit_observe_event(
    session_id: &str,
    cwd: Option<&str>,
    event: &'static str,
    payload: Value,
) {
    dispatch(session_id, cwd, event, PluginCapability::Observe, payload);
}

/// Deliver one tool event to plugins granted `tool_events`.
pub fn emit_tool_event(
    session_id: &str,
    cwd: Option<&str>,
    event: &'static str,
    payload: Value,
) {
    dispatch(session_id, cwd, event, PluginCapability::ToolEvents, payload);
}

fn dispatch(
    session_id: &str,
    cwd: Option<&str>,
    event: &'static str,
    capability: PluginCapability,
    payload: Value,
) {
    if !enabled() {
        return;
    }
    let Some(host) = session_host(session_id, cwd) else {
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        crate::logging::info(&format!(
            "Plugin event '{event}' skipped: no async runtime for session {session_id}"
        ));
        return;
    };
    let session_key = host.session_id.clone();
    handle.spawn(async move {
        let host = {
            let guard = runtime()
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.get(&session_key).map(Arc::clone)
        };
        let Some(host) = host else {
            return;
        };
        for slot in &host.slots {
            if !slot.config.capabilities.contains(&capability) {
                continue;
            }
            let Some(process) = host.ready(slot).await else {
                continue;
            };
            if !process.supports(capability) {
                continue;
            }
            let timeout = Duration::from_millis(slot.config.timeouts.event_ms.max(1));
            if let Err(error) = process.notify(event, payload.clone(), timeout).await {
                host.mark_unusable(slot, &error.to_string()).await;
            }
        }
    });
}

/// Collect context contributions for one turn, in configured plugin order.
pub async fn collect_context(session_id: &str, cwd: Option<&str>) -> Vec<PluginContribution> {
    if !enabled() {
        return Vec::new();
    }
    let Some(host) = session_host(session_id, cwd) else {
        return Vec::new();
    };
    let mut contributions = Vec::new();
    for slot in &host.slots {
        if !slot
            .config
            .capabilities
            .contains(&PluginCapability::Context)
        {
            continue;
        }
        let Some(process) = host.ready(slot).await else {
            continue;
        };
        if !process.supports(PluginCapability::Context) {
            continue;
        }
        let timeout = Duration::from_millis(slot.config.timeouts.context_ms.max(1));
        let params = json!({
            "session_id": session_id,
            "cwd": cwd,
            "plugin": slot.config.name,
            "max_bytes": protocol::CONTEXT_MAX_BYTES,
        });
        match process.request("context", params, timeout).await {
            Ok(result) => {
                let Some(text) = result.get("text").and_then(Value::as_str) else {
                    crate::logging::warn(&format!(
                        "Plugin '{}' returned a context response without text; ignoring it",
                        slot.config.name
                    ));
                    continue;
                };
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                let (text, truncated) = cap_contribution(text, &slot.config.name);
                contributions.push(PluginContribution {
                    plugin: slot.config.name.clone(),
                    text,
                    truncated,
                });
            }
            Err(error) => {
                let message = error.to_string();
                if message.contains("did not answer") {
                    crate::logging::warn(&format!(
                        "Plugin '{}' context contribution timed out after {}ms; continuing without it",
                        slot.config.name, timeout.as_millis()
                    ));
                } else {
                    crate::logging::warn(&format!(
                        "Plugin '{}' context contribution failed: {message}",
                        slot.config.name
                    ));
                    host.mark_unusable(slot, &message).await;
                }
            }
        }
    }
    contributions
}

fn cap_contribution(text: &str, plugin: &str) -> (String, bool) {
    if text.len() <= protocol::CONTEXT_MAX_BYTES {
        return (text.to_string(), false);
    }
    let mut end = protocol::CONTEXT_MAX_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    crate::logging::warn(&format!(
        "Plugin '{plugin}' contributed {} bytes, above the {} byte bound; truncating and recording it",
        text.len(),
        protocol::CONTEXT_MAX_BYTES
    ));
    (format!("{}\n[plugin contribution truncated at {} bytes]", &text[..end], protocol::CONTEXT_MAX_BYTES), true)
}

/// Run every granted gate in configured order. A `deny` stops the chain; a
/// rewrite is visible to later gates and is the input the caller executes.
pub async fn gate_tool(
    session_id: &str,
    cwd: Option<&str>,
    tool_name: &str,
    input: Value,
) -> GateOutcome {
    let mut current = input;
    if !enabled() {
        return GateOutcome {
            input: current,
            decision: GateDecision::Allow,
        };
    }
    let Some(host) = session_host(session_id, cwd) else {
        return GateOutcome {
            input: current,
            decision: GateDecision::Allow,
        };
    };
    for slot in &host.slots {
        if !slot.config.capabilities.contains(&PluginCapability::Gate) {
            continue;
        }
        let Some(process) = host.ready(slot).await else {
            crate::logging::warn(&format!(
                "Plugin '{}' gate skipped (plugin unusable); applying fallback {}",
                slot.config.name,
                fallback_label(slot.config.gate_fallback)
            ));
            if slot.config.gate_fallback == PluginGateFallback::Deny {
                return GateOutcome {
                    input: current,
                    decision: GateDecision::Deny(format!(
                        "denied: plugin '{}' is unusable and the configured gate fallback is fail-closed",
                        slot.config.name
                    )),
                };
            }
            continue;
        };
        if !process.supports(PluginCapability::Gate) {
            continue;
        }
        let timeout = Duration::from_millis(slot.config.timeouts.tool_gate_ms.max(1));
        let params = json!({
            "session_id": session_id,
            "cwd": cwd,
            "tool_name": tool_name,
            "input": current,
        });
        match process.request("gate", params, timeout).await {
            Ok(result) => match result.get("decision").and_then(Value::as_str) {
                Some("allow") => {}
                Some("deny") => {
                    let reason = result
                        .get("reason")
                        .and_then(Value::as_str)
                        .filter(|reason| !reason.trim().is_empty())
                        .unwrap_or("denied by plugin gate")
                        .to_string();
                    return GateOutcome {
                        input: current,
                        decision: GateDecision::Deny(format!(
                            "denied by plugin '{}': {reason}",
                            slot.config.name
                        )),
                    };
                }
                Some("rewrite") => {
                    let Some(replacement) = result.get("input").cloned() else {
                        crate::logging::warn(&format!(
                            "Plugin '{}' said rewrite but returned no input; applying fallback {}",
                            slot.config.name,
                            fallback_label(slot.config.gate_fallback)
                        ));
                        if slot.config.gate_fallback == PluginGateFallback::Deny {
                            return GateOutcome {
                                input: current,
                                decision: GateDecision::Deny(format!(
                                    "denied: plugin '{}' returned an unusable rewrite and the fallback is fail-closed",
                                    slot.config.name
                                )),
                            };
                        }
                        continue;
                    };
                    current = replacement;
                }
                other => {
                    crate::logging::warn(&format!(
                        "Plugin '{}' gate returned unknown decision {other:?}; applying fallback {}",
                        slot.config.name,
                        fallback_label(slot.config.gate_fallback)
                    ));
                    if slot.config.gate_fallback == PluginGateFallback::Deny {
                        return GateOutcome {
                            input: current,
                            decision: GateDecision::Deny(format!(
                                "denied: plugin '{}' returned no usable decision and the fallback is fail-closed",
                                slot.config.name
                            )),
                        };
                    }
                }
            },
            Err(error) => {
                let message = error.to_string();
                let timed_out = message.contains("did not answer");
                if !timed_out {
                    host.mark_unusable(slot, &message).await;
                }
                crate::logging::warn(&format!(
                    "Plugin '{}' gate {}: {message}; applying fallback {}",
                    slot.config.name,
                    if timed_out { "timed out" } else { "failed" },
                    fallback_label(slot.config.gate_fallback)
                ));
                if slot.config.gate_fallback == PluginGateFallback::Deny {
                    return GateOutcome {
                        input: current,
                        decision: GateDecision::Deny(format!(
                            "denied: plugin '{}' gate did not answer and the fallback is fail-closed",
                            slot.config.name
                        )),
                    };
                }
            }
        }
    }
    GateOutcome {
        input: current,
        decision: GateDecision::Allow,
    }
}

fn fallback_label(fallback: PluginGateFallback) -> &'static str {
    match fallback {
        PluginGateFallback::Allow => "allow",
        PluginGateFallback::Deny => "deny",
    }
}

/// Ask every tool-capable plugin for its tools and validate the declarations.
/// `taken` is the set of names already offered by jcode; a plugin tool never
/// shadows one of those.
pub async fn plugin_tool_specs(
    session_id: &str,
    cwd: Option<&str>,
    taken: &HashSet<String>,
) -> Vec<PluginToolSpec> {
    if !enabled() {
        return Vec::new();
    }
    let Some(host) = session_host(session_id, cwd) else {
        return Vec::new();
    };
    let mut specs = Vec::new();
    let mut names: HashSet<String> = taken.clone();
    for slot in &host.slots {
        if !slot.config.capabilities.contains(&PluginCapability::Tools) {
            continue;
        }
        let Some(process) = host.ready(slot).await else {
            continue;
        };
        if !process.supports(PluginCapability::Tools) {
            continue;
        }
        let timeout = Duration::from_millis(slot.config.timeouts.tool_call_ms.max(1));
        let params = json!({ "session_id": session_id, "cwd": cwd, "plugin": slot.config.name });
        let result = match process.request("tools/list", params, timeout).await {
            Ok(result) => result,
            Err(error) => {
                let message = error.to_string();
                crate::logging::warn(&format!(
                    "Plugin '{}' tool listing failed: {message}",
                    slot.config.name
                ));
                if !message.contains("did not answer") {
                    host.mark_unusable(slot, &message).await;
                }
                continue;
            }
        };
        let Some(tools) = result.get("tools").and_then(Value::as_array) else {
            crate::logging::warn(&format!(
                "Plugin '{}' tool listing returned no tools array; ignoring it",
                slot.config.name
            ));
            continue;
        };
        for tool in tools {
            let Some(name) = tool.get("name").and_then(Value::as_str).map(str::trim) else {
                crate::logging::warn(&format!(
                    "Plugin '{}' offered a tool without a name; refusing it",
                    slot.config.name
                ));
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let Some(description) = tool.get("description").and_then(Value::as_str) else {
                crate::logging::warn(&format!(
                    "Plugin '{}' tool '{name}' has no description; refusing it",
                    slot.config.name
                ));
                continue;
            };
            let Some(schema) = tool.get("input_schema") else {
                crate::logging::warn(&format!(
                    "Plugin '{}' tool '{name}' has no input schema; refusing it",
                    slot.config.name
                ));
                continue;
            };
            if !schema.is_object() {
                crate::logging::warn(&format!(
                    "Plugin '{}' tool '{name}' has a malformed input schema; refusing it",
                    slot.config.name
                ));
                continue;
            }
            let flattened = format!("{}{}{}", slot.config.name, protocol::TOOL_NAME_SEPARATOR, name);
            if names.contains(&flattened) {
                crate::logging::warn(&format!(
                    "Plugin '{}' tool '{name}' collides with an existing tool '{flattened}'; refusing it",
                    slot.config.name
                ));
                continue;
            }
            names.insert(flattened.clone());
            specs.push(PluginToolSpec {
                flattened_name: flattened,
                plugin: slot.config.name.clone(),
                name: name.to_string(),
                description: description.to_string(),
                input_schema: schema.clone(),
            });
        }
    }
    specs
}

/// Whether `name` is a model-facing plugin tool (contains the separator and a
/// configured plugin prefix).
pub fn is_plugin_tool_name(name: &str) -> bool {
    split_plugin_tool_name(name).is_some()
}

/// Split a model-facing plugin tool name into `(plugin, original tool name)`.
pub fn split_plugin_tool_name(name: &str) -> Option<(String, String)> {
    let (prefix, rest) = name.split_once(protocol::TOOL_NAME_SEPARATOR)?;
    if rest.is_empty() {
        return None;
    }
    configured_plugins()
        .iter()
        .any(|plugin| plugin.name == prefix)
        .then(|| (prefix.to_string(), rest.to_string()))
}

/// Execute a plugin tool through its owning plugin.
pub async fn call_plugin_tool(
    session_id: &str,
    cwd: Option<&str>,
    flattened_name: &str,
    input: Value,
) -> anyhow::Result<String> {
    let (plugin, name) = split_plugin_tool_name(flattened_name)
        .ok_or_else(|| anyhow::anyhow!("'{flattened_name}' is not a plugin tool"))?;
    let Some(host) = session_host(session_id, cwd) else {
        anyhow::bail!("plugin session '{session_id}' is not registered");
    };
    let slot = host
        .slots
        .iter()
        .find(|slot| slot.config.name == plugin)
        .ok_or_else(|| anyhow::anyhow!("plugin '{plugin}' is not configured"))?;
    let Some(process) = host.ready(slot).await else {
        anyhow::bail!("plugin '{plugin}' is unusable this session");
    };
    let timeout = Duration::from_millis(slot.config.timeouts.tool_call_ms.max(1));
    let params = json!({
        "session_id": session_id,
        "cwd": cwd,
        "name": name,
        "input": input,
    });
    let result = process.request("tools/call", params, timeout).await?;
    if let Some(output) = result.get("output").and_then(Value::as_str) {
        return Ok(output.to_string());
    }
    Ok(result.to_string())
}

/// Remove a session from the registry and tombstone it, under one lock so a
/// concurrent extension-point call cannot revive a torn-down session.
fn take_session_host(session_id: &str) -> Option<Arc<SessionHost>> {
    runtime()
        .ended
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(session_id.to_string());
    runtime()
        .sessions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(session_id)
}

/// Deliver `session_end` (bounded by the configured `event_ms`) and kill every
/// plugin process. Delivery is awaited so a caller that is about to exit can
/// guarantee the last event was written; see [`session_ended_async`].
async fn teardown_host(host: Arc<SessionHost>, payload: Value) {
    for slot in &host.slots {
        let state = slot.state.lock().await;
        if let SlotState::Ready(process) = &*state {
            let observes = process.supports(PluginCapability::Observe)
                && slot
                    .config
                    .capabilities
                    .contains(&PluginCapability::Observe);
            let mut grace = Duration::ZERO;
            if observes {
                let timeout = Duration::from_millis(slot.config.timeouts.event_ms.max(1));
                let _ = process.notify("session_end", payload.clone(), timeout).await;
                grace = timeout;
            }
            // The event is only in the pipe at this point. Closing stdin gives
            // the plugin EOF so it can read the last line, flush, and exit;
            // killing first would discard unread data. Teardown stays bounded
            // by the same event timeout, then falls back to a hard kill.
            process.close_input().await;
            if grace > Duration::ZERO && process.wait_for_exit(grace).await {
                continue;
            }
            process.kill().await;
        }
    }
}

/// Awaitable teardown: `session_end` has been written to every subscribed plugin
/// pipe before this returns.
///
/// Callers that are about to exit the process **must** use this. The
/// fire-and-forget [`session_ended`] spawns a detached task, and a process that
/// exits immediately afterwards kills the child before that task runs, silently
/// dropping the final event.
pub async fn session_ended_async(session_id: &str, payload: Value) {
    if !enabled() {
        return;
    }
    let Some(host) = take_session_host(session_id) else {
        return;
    };
    teardown_host(host, payload).await;
}

/// Emit `session_end` and tear every process down, without waiting.
///
/// Only safe for a caller that keeps its runtime alive after this returns (a
/// long-lived daemon, or a reconnect). A process that exits — a one-shot
/// `jcode run` — loses the event, so exit paths use [`session_ended_async`].
pub fn session_ended(session_id: &str, payload: Value) {
    if !enabled() {
        return;
    }
    let Some(host) = take_session_host(session_id) else {
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        // Dropping the host drops each child with kill_on_drop set.
        crate::logging::info(&format!(
            "Plugin session {session_id} torn down without an async runtime"
        ));
        return;
    };
    handle.spawn(teardown_host(host, payload));
}

/// Start a plugin, handshake, report, and stop it. Used by `jcode plugin list`
/// and `jcode plugin test` and requires no session or model call.
pub async fn probe_plugin(config: &PluginConfig) -> PluginProbeReport {
    match host::spawn_plugin(config, "plugin-probe", None).await {
        Ok(process) => {
            let report = PluginProbeReport {
                name: config.name.clone(),
                usable: true,
                reported_name: Some(process.reported_name().to_string()),
                version: Some(process.version().to_string()),
                protocol: Some(process.protocol_version()),
                declared: process.declared().to_vec(),
                granted: process.granted().to_vec(),
                error: None,
            };
            process.kill().await;
            report
        }
        Err(error) => PluginProbeReport {
            name: config.name.clone(),
            usable: false,
            reported_name: None,
            version: None,
            protocol: None,
            declared: Vec::new(),
            granted: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contribution_cap_is_visible_in_the_text() {
        let big = "x".repeat(protocol::CONTEXT_MAX_BYTES + 10);
        let (text, truncated) = cap_contribution(&big, "p");
        assert!(truncated);
        assert!(text.contains("truncated at"), "{text}");
        assert!(text.len() <= protocol::CONTEXT_MAX_BYTES + 128);
    }

    #[test]
    fn gate_outcome_defaults_to_allow_without_plugins() {
        // `enabled()` is false in this unit context because no config is set,
        // so the no-plugin path must return the input untouched.
        let outcome = futures::executor::block_on(gate_tool("ses", None, "bash", json!({"a": 1})));
        assert!(matches!(outcome.decision, GateDecision::Allow));
        assert_eq!(outcome.input, json!({"a": 1}));
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// Minimal shell plugin used to exercise the real child-process path.
    /// Mode selects context behaviour; gate_action selects the gate answer;
    /// the log file records delivered events and gate calls.
    const PLUGIN_SCRIPT: &str = r#"#!/bin/sh
MODE="$1"
LOG="$2"
GATE="$3"
trap 'printf "exited\n" >> "$LOG"' EXIT
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"type":"hello"'*)
      printf '%s\n' '{"type":"hello","protocol":1,"name":"shell-plugin","version":"1.0.0","capabilities":["observe","tool_events","context","tools","gate"]}'
      if [ "$MODE" = "die-after-hello" ]; then exit 0; fi
      ;;
    *'"type":"event"'*)
      if [ "$MODE" = "stuck" ]; then continue; fi
      printf '%s\n' "$line" >> "$LOG"
      ;;
    *'"method":"context"'*)
      if [ "$MODE" = "garbage" ]; then printf '%s\n' 'not protocol json'; fi
      if [ "$MODE" = "silent" ]; then continue; fi
      if [ "$MODE" = "empty" ]; then
        printf '{"type":"response","id":%s,"ok":true,"result":{"text":""}}\n' "$id"
        continue
      fi
      printf '{"type":"response","id":%s,"ok":true,"result":{"text":"ctx-%s"}}\n' "$id" "$MODE"
      ;;
    *'"method":"gate"'*)
      printf 'gate %s\n' "$line" >> "$LOG"
      if [ "$GATE" = "silent" ]; then continue; fi
      case "$GATE" in
        deny) printf '{"type":"response","id":%s,"ok":true,"result":{"decision":"deny","reason":"denied by %s"}}\n' "$id" "$MODE" ;;
        rewrite) printf '{"type":"response","id":%s,"ok":true,"result":{"decision":"rewrite","input":{"command":"rewritten"}}}\n' "$id" ;;
        deny-rewritten)
          case "$line" in
            *'"command":"rewritten"'*) printf '{"type":"response","id":%s,"ok":true,"result":{"decision":"deny","reason":"saw rewritten input"}}\n' "$id" ;;
            *) printf '{"type":"response","id":%s,"ok":true,"result":{"decision":"allow"}}\n' "$id" ;;
          esac
          ;;
        *) printf '{"type":"response","id":%s,"ok":true,"result":{"decision":"allow"}}\n' "$id" ;;
      esac
      ;;
    *'"method":"tools/list"'*)
      if [ "$MODE" = "bad-schema" ]; then
        printf '{"type":"response","id":%s,"ok":true,"result":{"tools":[{"name":"broken","description":"d","input_schema":"nope"}]}}\n' "$id"
      else
        printf '{"type":"response","id":%s,"ok":true,"result":{"tools":[{"name":"echo","description":"echo input","input_schema":{"type":"object"}}]}}\n' "$id"
      fi
      ;;
    *'"method":"tools/call"'*)
      printf '{"type":"response","id":%s,"ok":true,"result":{"output":"called-%s"}}\n' "$id" "$MODE"
      ;;
  esac
done
"#;

    struct TempConfig {
        _lock: std::sync::MutexGuard<'static, ()>,
        temp: tempfile::TempDir,
        previous_home: Option<std::ffi::OsString>,
    }

    impl TempConfig {
        fn new(plugins: &[(&str, &str, &str, &str, &str)]) -> Self {
            // (name, mode, gate_action, capabilities, extra_toml)
            let lock = crate::storage::lock_test_env();
            let temp = tempfile::tempdir().expect("tempdir");
            let home = temp.path();
            let mut toml = String::new();
            for (name, mode, gate, capabilities, extra) in plugins {
                let script = write_plugin_script(home, name);
                let log = home.join(format!("{name}.log"));
                toml.push_str(&format!(
                    "[[plugins]]\nname = \"{name}\"\ncommand = \"{}\"\nargs = [\"{mode}\", \"{}\", \"{gate}\"]\ncapabilities = [{capabilities}]\n{extra}\n",
                    script.display(),
                    log.display()
                ));
            }
            std::fs::write(home.join("config.toml"), toml).expect("write config");
            let previous_home = std::env::var_os("JCODE_HOME");
            crate::env::set_var("JCODE_HOME", home);
            crate::config::invalidate_config_cache();
            Self {
                _lock: lock,
                temp,
                previous_home,
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.temp.path().join(name)
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            crate::config::invalidate_config_cache();
            match self.previous_home.take() {
                Some(value) => crate::env::set_var("JCODE_HOME", value),
                None => crate::env::remove_var("JCODE_HOME"),
            }
            crate::config::invalidate_config_cache();
        }
    }

    fn write_plugin_script(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.sh"));
        std::fs::write(&path, PLUGIN_SCRIPT).expect("write plugin script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod plugin script");
        path
    }

    async fn wait_for_log(path: &Path, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if std::fs::read_to_string(path)
                .map(|text| text.contains(needle))
                .unwrap_or(false)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out waiting for {needle:?} in {path:?}");
    }

    /// Documents the defect the awaited path fixes: a detached teardown task is
    /// not guaranteed to run before a one-shot process exits.
    #[test]
    fn detached_teardown_loses_session_end_when_the_runtime_exits_immediately() {
        let config = TempConfig::new(&[(
            "detached",
            "detached",
            "allow",
            "\"observe\", \"context\"",
            "",
        )]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            // Start the process so a session host exists, and prove it is live.
            let contributions = collect_context("ses_detached_loss", None).await;
            assert_eq!(contributions.len(), 1, "the plugin process must be running");
            // Exactly the headless shape: fire the event, then leave without
            // yielding. The spawned delivery task has not been polled yet.
            session_ended("ses_detached_loss", json!({"event": "session_end"}));
        });
        drop(rt);
        std::thread::sleep(Duration::from_millis(100));
        let log = std::fs::read_to_string(config.path("detached.log")).unwrap_or_default();
        assert!(
            !log.contains("session_end"),
            "detached delivery must not be relied on for exit paths: {log:?}"
        );
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn awaited_teardown_delivers_session_end_before_returning() {
        let config = TempConfig::new(&[(
            "awaited",
            "awaited",
            "allow",
            "\"observe\", \"context\"",
            "",
        )]);
        let _ = collect_context("ses_awaited", None).await;
        session_ended_async("ses_awaited", json!({"event": "session_end"})).await;
        let log = std::fs::read_to_string(config.path("awaited.log")).unwrap_or_default();
        assert!(
            log.contains("session_end"),
            "an awaited teardown must have written the event before returning: {log:?}"
        );
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn awaited_teardown_is_bounded_by_the_event_timeout() {
        let config = TempConfig::new(&[(
            "stuck",
            "stuck",
            "allow",
            "\"observe\", \"context\"",
            "[plugins.timeouts]\nevent_ms = 200\n",
        )]);
        // A plugin that stops reading stdin makes the write block; the event
        // timeout must bound teardown rather than hanging the exit.
        let _ = collect_context("ses_stuck", None).await;
        let started = Instant::now();
        session_ended_async("ses_stuck", json!({"event": "session_end"})).await;
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "teardown took {:?}, above its bound",
            started.elapsed()
        );
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn context_contributions_follow_configured_order() {
        let config = TempConfig::new(&[
            ("alpha", "alpha", "allow", "\"context\"", ""),
            ("beta", "beta", "allow", "\"context\"", ""),
        ]);
        let contributions = collect_context("ses_order", None).await;
        let texts: Vec<_> = contributions.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["ctx-alpha", "ctx-beta"]);
        // Stable across turns.
        let again = collect_context("ses_order", None).await;
        assert_eq!(
            again.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            texts
        );
        session_ended("ses_order", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gates_compose_in_order_and_later_gate_sees_the_rewrite() {
        let config = TempConfig::new(&[
            ("first", "first", "rewrite", "\"gate\"", ""),
            ("second", "second", "deny-rewritten", "\"gate\"", ""),
        ]);
        let outcome = gate_tool(
            "ses_gate_chain",
            None,
            "bash",
            json!({"command": "original"}),
        )
        .await;
        match outcome.decision {
            GateDecision::Deny(reason) => {
                assert!(reason.contains("saw rewritten input"), "{reason}");
                assert!(reason.contains("second"), "{reason}");
            }
            GateDecision::Allow => panic!("the second gate must deny the rewritten input"),
        }
        assert_eq!(outcome.input, json!({"command": "rewritten"}));
        session_ended("ses_gate_chain", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gate_timeout_applies_the_configured_fallback() {
        let config = TempConfig::new(&[(
            "silent",
            "silent",
            "silent",
            "\"gate\"",
            "gate_fallback = \"deny\"\n[plugins.timeouts]\ntool_gate_ms = 150\n",
        )]);
        let outcome = gate_tool("ses_gate_timeout", None, "bash", json!({"command": "x"})).await;
        match outcome.decision {
            GateDecision::Deny(reason) => {
                assert!(reason.contains("fail-closed"), "{reason}");
                assert!(reason.contains("silent"), "{reason}");
            }
            GateDecision::Allow => panic!("the configured fail-closed fallback must deny"),
        }
        session_ended("ses_gate_timeout", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn capability_grants_limit_what_each_plugin_is_asked() {
        let config = TempConfig::new(&[
            ("observer", "observer", "allow", "\"observe\"", ""),
            ("contextor", "contextor", "allow", "\"context\"", ""),
        ]);
        // Only the context-capable plugin contributes.
        let contributions = collect_context("ses_caps", None).await;
        assert_eq!(contributions.len(), 1);
        assert_eq!(contributions[0].plugin, "contextor");

        // Only observe-capable plugins receive lifecycle events; the context
        // plugin is not asked for events it did not grant.
        emit_observe_event(
            "ses_caps",
            None,
            "turn_start",
            json!({"event": "turn_start"}),
        );
        wait_for_log(&config.path("observer.log"), "turn_start").await;
        assert!(
            !std::fs::read_to_string(config.path("contextor.log")).unwrap_or_default().contains("turn_start"),
            "a plugin without the observe grant must not receive lifecycle events"
        );

        // A plugin without the gate grant is never asked to decide a call.
        let outcome = gate_tool("ses_caps", None, "bash", json!({"command": "x"})).await;
        assert!(matches!(outcome.decision, GateDecision::Allow));
        assert!(
            !config.path("observer.log").exists()
                || !std::fs::read_to_string(config.path("observer.log"))
                    .unwrap_or_default()
                    .contains("gate "),
            "an observe-only plugin must not be asked to gate a tool call"
        );
        session_ended("ses_caps", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn plugin_tools_are_prefixed_and_bad_or_colliding_tools_are_refused() {
        let config = TempConfig::new(&[
            ("good", "good", "allow", "\"tools\"", ""),
            ("bad", "bad-schema", "allow", "\"tools\"", ""),
        ]);
        let mut taken = HashSet::new();
        taken.insert("good__echo".to_string());
        let specs = plugin_tool_specs("ses_tools", None, &taken).await;
        assert!(
            specs.is_empty(),
            "a colliding tool and a malformed schema must both be refused: {specs:?}"
        );

        let specs = plugin_tool_specs("ses_tools_2", None, &HashSet::new()).await;
        assert_eq!(specs.len(), 1, "{specs:?}");
        assert_eq!(specs[0].flattened_name, "good__echo");
        let output = call_plugin_tool("ses_tools_2", None, "good__echo", json!({}))
            .await
            .expect("plugin tool executes");
        assert_eq!(output, "called-good");
        session_ended("ses_tools", json!({}));
        session_ended("ses_tools_2", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn context_timeout_degrades_without_extending_the_turn() {
        let config = TempConfig::new(&[(
            "slow",
            "silent",
            "allow",
            "\"context\"",
            "[plugins.timeouts]\ncontext_ms = 150\n",
        )]);
        let started = Instant::now();
        let contributions = collect_context("ses_context_timeout", None).await;
        assert!(contributions.is_empty());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "a silent context contribution must not extend the turn beyond its timeout"
        );
        session_ended("ses_context_timeout", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_empty_contribution_adds_nothing() {
        let config = TempConfig::new(&[("empty", "empty", "allow", "\"context\"", "")]);
        assert!(collect_context("ses_empty", None).await.is_empty());
        session_ended("ses_empty", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_plugin_that_dies_is_marked_unusable_and_never_restarted() {
        let config = TempConfig::new(&[("dies", "die-after-hello", "allow", "\"context\"", "")]);
        let first = collect_context("ses_dead", None).await;
        assert!(first.is_empty(), "a dead plugin contributes nothing");
        let second = collect_context("ses_dead", None).await;
        assert!(second.is_empty());
        // The process exited after its hello; both calls contributed nothing,
        // so it was never restarted within the session.
        session_ended("ses_dead", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_end_kills_the_process_and_does_not_revive_it() {
        let config = TempConfig::new(&[("teardown", "teardown", "allow", "\"context\"", "")]);
        assert_eq!(collect_context("ses_teardown", None).await.len(), 1);
        session_ended("ses_teardown", json!({}));
        tokio::time::sleep(Duration::from_millis(100)).await;
        // A late call after teardown must not spawn a replacement process.
        assert!(collect_context("ses_teardown", None).await.is_empty());
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "manual overhead measurement; not a pass/fail test"]
    async fn measure_plugin_overhead() {
        let config = TempConfig::new(&[("measure", "measure", "allow", "\"context\"", "")]);
        let first = std::time::Instant::now();
        let _ = collect_context("ses_measure", None).await;
        eprintln!("MEASURE first_use_ms={}", first.elapsed().as_millis());
        let mut samples = Vec::new();
        for _ in 0..5 {
            let started = std::time::Instant::now();
            let _ = collect_context("ses_measure", None).await;
            samples.push(started.elapsed().as_micros());
        }
        eprintln!("MEASURE context_roundtrip_us={samples:?}");
        session_ended("ses_measure", json!({}));
        drop(config);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_protocol_output_is_contained_during_a_request() {
        let config = TempConfig::new(&[("garbage", "garbage", "allow", "\"context\"", "")]);
        let contributions = collect_context("ses_garbage", None).await;
        assert_eq!(contributions.len(), 1);
        assert_eq!(contributions[0].text, "ctx-garbage");
        session_ended("ses_garbage", json!({}));
        drop(config);
    }
}
