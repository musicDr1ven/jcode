//! Extension plugin host.
//!
//! Plugins are configured with `[[plugins]]`, launched as long-lived child
//! processes on first use in a session, and spoken to over line-delimited JSON
//! on stdio. Four extension points are supported: lifecycle observation, context
//! contribution, tool registration, and tool-call gating. With no plugin
//! configured every public helper here returns immediately, so the ordinary
//! path is unchanged.
//!
//! The protocol and the trust model are documented in `docs/PLUGINS.md`.

pub(crate) mod host;
mod manager;
pub(crate) mod protocol;

pub use manager::{
    GateDecision, GateOutcome, PluginContribution, PluginProbeReport, PluginToolSpec,
    call_plugin_tool, collect_context, emit_observe_event, emit_tool_event, enabled, gate_tool,
    is_plugin_tool_name, plugin_tool_specs, probe_plugin, session_ended, session_started,
    split_plugin_tool_name,
};
pub use crate::config::PluginCapability;
pub use protocol::{
    CONTEXT_MAX_BYTES, EVENT_PAYLOAD_MAX_BYTES, PROTOCOL_NAME, PROTOCOL_VERSION,
    TOOL_NAME_SEPARATOR,
};
