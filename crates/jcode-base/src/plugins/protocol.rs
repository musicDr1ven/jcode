//! Wire protocol shared by the jcode plugin host and plugin processes.
//!
//! One JSON object per line, over the plugin's stdin/stdout. Version 1 is the
//! only version this build speaks; a plugin that answers with another version is
//! refused rather than used partially. See `docs/PLUGINS.md`.

use serde_json::{Value, json};

/// Protocol identifier reported in diagnostics.
pub const PROTOCOL_NAME: &str = "jcode-plugin";

/// The protocol version this build speaks. Bump only for a breaking change.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum bytes of a single event payload forwarded to a plugin. Larger
/// payloads are reduced to an envelope that says they were capped.
pub const EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;

/// Maximum bytes of context one plugin may contribute in a turn.
pub const CONTEXT_MAX_BYTES: usize = 16 * 1024;

/// Separator between a plugin name and a plugin tool name in the model-facing
/// tool list (`reference__ping`). Plugin names may not contain it.
pub const TOOL_NAME_SEPARATOR: &str = "__";

/// An outbound request, correlated by `id`.
pub fn request(id: u64, method: &str, params: Value) -> Value {
    json!({
        "protocol": PROTOCOL_VERSION,
        "type": "request",
        "id": id,
        "method": method,
        "params": params,
    })
}

/// The opening handshake. The plugin answers with `protocol`, `name`,
/// `version`, and the capabilities it supports.
pub fn hello(id: u64, session_id: &str, cwd: Option<&str>, granted: &[&str]) -> Value {
    json!({
        "protocol": PROTOCOL_VERSION,
        "type": "hello",
        "id": id,
        "protocol_versions": [PROTOCOL_VERSION],
        "session_id": session_id,
        "cwd": cwd,
        "granted_capabilities": granted,
    })
}

/// A fire-and-forget lifecycle notification (no response expected).
pub fn event(event: &str, payload: Value) -> Value {
    json!({
        "protocol": PROTOCOL_VERSION,
        "type": "event",
        "event": event,
        "payload": payload,
    })
}

/// Whether a parsed hello is compatible, or why it is not. The error names both
/// versions, as the spec requires.
pub fn check_hello_protocol(value: &Value) -> Result<(), String> {
    let declared = value.get("protocol").and_then(Value::as_u64);
    match declared {
        Some(version) if version == u64::from(PROTOCOL_VERSION) => Ok(()),
        Some(version) => Err(format!(
            "plugin '{}' speaks protocol v{version}, but this jcode speaks v{PROTOCOL_VERSION}",
            value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("<unnamed>")
        )),
        None => Err(format!(
            "plugin '{}' did not declare a protocol version; this jcode speaks v{PROTOCOL_VERSION}",
            value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("<unnamed>")
        )),
    }
}

/// Cap a payload at [`EVENT_PAYLOAD_MAX_BYTES`], replacing it with a small
/// envelope when it is larger so the cap is visible instead of silent.
pub fn cap_payload(event: &str, payload: Value) -> (Value, bool) {
    let serialized = serde_json::to_vec(&payload).unwrap_or_default();
    if serialized.len() <= EVENT_PAYLOAD_MAX_BYTES {
        return (payload, false);
    }
    (
        json!({
            "event": event,
            "truncated": true,
            "original_bytes": serialized.len(),
            "cap_bytes": EVENT_PAYLOAD_MAX_BYTES,
        }),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_accepts_matching_version_and_names_both_when_refusing() {
        assert!(check_hello_protocol(&json!({"protocol": PROTOCOL_VERSION})).is_ok());
        let error = check_hello_protocol(&json!({"protocol": 99, "name": "old"}))
            .expect_err("v99 must be refused");
        assert!(error.contains("v99") && error.contains("v1"), "{error}");
    }

    #[test]
    fn oversized_payload_is_replaced_with_a_visible_cap_envelope() {
        let big = "x".repeat(EVENT_PAYLOAD_MAX_BYTES + 1);
        let (capped, truncated) = cap_payload("turn_end", json!({ "last": big }));
        assert!(truncated);
        assert_eq!(capped["truncated"], json!(true));
        assert_eq!(capped["cap_bytes"], json!(EVENT_PAYLOAD_MAX_BYTES));
    }
}
