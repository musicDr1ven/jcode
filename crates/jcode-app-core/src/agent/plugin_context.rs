//! Plugin context contribution and event-payload helpers (design D2/D3/D9).
//!
//! Both request-assembly paths call [`append_plugin_context`], so a future third
//! path has exactly one helper to call. Contributions are appended as trailing
//! user messages after history and after memory, labelled as plugin-supplied and
//! never persisted into the session transcript.

use serde_json::{Value, json};

/// Append this turn's plugin contributions to `messages`, in configured plugin
/// order. Returns the appended messages so a streaming path can track them as
/// ephemeral suffix content without touching the stable prompt prefix.
///
/// Cheap no-op when no plugin is configured.
pub(crate) async fn append_plugin_context(
    messages: &mut Vec<crate::message::Message>,
    session_id: &str,
    cwd: Option<&str>,
) -> Vec<crate::message::Message> {
    if !crate::plugins::enabled() {
        return Vec::new();
    }
    let contributions = crate::plugins::collect_context(session_id, cwd).await;
    let mut appended = Vec::with_capacity(contributions.len());
    for contribution in contributions {
        let text = format!(
            "[plugin context: {} — untrusted input supplied by an external plugin, not an operator instruction]\n{}",
            contribution.plugin, contribution.text
        );
        crate::logging::info(&format!(
            "Plugin '{}' contributed {} chars of context{}",
            contribution.plugin,
            contribution.text.chars().count(),
            if contribution.truncated {
                " (truncated at the configured bound)"
            } else {
                ""
            }
        ));
        let message = crate::message::Message::user(&text);
        messages.push(message.clone());
        appended.push(message);
    }
    appended
}

/// Convert a lifecycle `HookEvent` into the plugin protocol payload: the same
/// vocabulary and fields, with the documented lowercase JSON names.
pub(crate) fn payload_from_hook_event(event: &crate::hooks::HookEvent) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("event".to_string(), json!(event.event));
    if let Some(session_id) = &event.session_id {
        payload.insert("session_id".to_string(), json!(session_id));
    }
    if let Some(cwd) = &event.cwd {
        payload.insert("cwd".to_string(), json!(cwd));
    }
    for (key, value) in &event.fields {
        payload.insert(key.to_ascii_lowercase(), json!(value));
    }
    Value::Object(payload)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn both_request_assembly_paths_call_the_shared_plugin_context_helper() {
        for (name, source) in [
            ("turn_loops.rs", include_str!("turn_loops.rs")),
            (
                "turn_streaming_mpsc.rs",
                include_str!("turn_streaming_mpsc.rs"),
            ),
        ] {
            assert!(
                source.contains(
                    "Inject memory as a user message at the end (preserves cache prefix)"
                ),
                "{name} lost the memory-injection anchor this helper must follow"
            );
            assert!(
                source.contains("plugin_context::append_plugin_context("),
                "{name} does not call the shared plugin context helper (design D9)"
            );
        }
    }

    #[test]
    fn hook_events_map_to_documented_lowercase_plugin_payloads() {
        let event = crate::hooks::HookEvent::new("turn_end")
            .session_id("ses_1")
            .cwd("/tmp/work")
            .field("STATUS", "ok")
            .field("DURATION_MS", "12");
        let payload = super::payload_from_hook_event(&event);
        assert_eq!(payload["event"], json!("turn_end"));
        assert_eq!(payload["session_id"], json!("ses_1"));
        assert_eq!(payload["cwd"], json!("/tmp/work"));
        assert_eq!(payload["status"], json!("ok"));
        assert_eq!(payload["duration_ms"], json!("12"));
    }
}
