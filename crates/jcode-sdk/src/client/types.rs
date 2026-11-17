//! Public client operation options and response types.

use super::*;

/// Options for a one-shot turn.
#[derive(Default)]
pub struct RunOptions {
    pub images: Vec<(String, String)>,
    /// Called for every event of the turn, for progress rendering.
    #[allow(clippy::type_complexity)]
    pub on_event: Option<Box<dyn Fn(&ApiEvent) + Send>>,
    /// Auto-answer permission prompts. Only meaningful when the server
    /// advertises the `permissions` capability.
    pub auto_approve: bool,
}

/// Runtime identity and protocol metadata returned by [`JcodeClient::get_runtime_info`].
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeInfo {
    pub server: String,
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
    pub healthy: bool,
    pub session_id: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    /// Reasoning effort, e.g. `high`, when the provider exposes it.
    pub reasoning_effort: Option<String>,
    /// Credential the session bills against (`oauth` or `api_key`).
    pub auth_method: Option<String>,
    /// OAuth account label the daemon's account router selected, if any.
    pub account_label: Option<String>,
    pub providers: Vec<String>,
    pub routes: Vec<ModelRouteInfo>,
}

/// Content and truncation metadata returned by [`JcodeClient::read_file`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub size: u64,
    pub truncated: bool,
}

/// Optional constraints for [`JcodeClient::search_text`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchTextOptions {
    /// Restrict the search to this relative path under the session root.
    pub path: Option<String>,
    /// Maximum number of matches to return.
    pub limit: Option<u32>,
}

/// Safe filesystem metadata returned by [`JcodeClient::file_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub exists: bool,
    pub kind: String,
    pub size: Option<u64>,
    pub modified_ms: Option<u64>,
}

/// What one turn produced.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TurnResult {
    /// None for natural completion. Failures still return Err and are also
    /// delivered to on_event as TurnStopped before the legacy Error event.
    pub stop_reason: Option<jcode_harness_api::TurnStopReason>,
    pub stop_message: Option<String>,
    /// All assistant text in the turn, including tool narration.
    pub text: String,
    /// Last completed assistant message, or aggregate text on older bridges.
    pub final_text: String,
    /// Framed messages. Empty when connected to an older, unframed bridge.
    pub messages: Vec<AssistantTextMessage>,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    /// Usage from the latest provider call in this turn, not a sum of calls.
    pub usage: Option<Usage>,
}

/// One assistant text message, excluding interleaved reasoning.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistantTextMessage {
    /// Stream-local correlator, not a persisted history message id.
    pub message_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub output: String,
    pub error: Option<String>,
}

/// Provider-reported counters. Cache counters may be separate from input
/// (Anthropic) or a subset of it (OpenAI), so do not blindly add them together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read_input: Option<u64>,
    pub cache_creation_input: Option<u64>,
    pub effective_budget: Option<usize>,
}

/// A named OpenAI-compatible provider profile to create or replace on the
/// runtime host. Only `name`, `base_url`, and `model` are required.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderProfileSpec {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub context_window: Option<usize>,
    pub api_key_env: Option<String>,
    /// One-line API key stored in Jcode's owner-only env file. Never logged.
    pub api_key: Option<String>,
    pub env_file: Option<String>,
    pub no_api_key: bool,
    /// `bearer` (default), `header`, or `none`.
    pub auth: Option<String>,
    pub auth_header: Option<String>,
    pub set_default: bool,
    pub overwrite: bool,
    pub provider_routing: bool,
    pub model_catalog: bool,
}
