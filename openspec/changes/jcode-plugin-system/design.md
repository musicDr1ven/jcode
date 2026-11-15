# Design

## Context

Constraints verified in this workspace (~90 crates), which shape every decision below:

- **There is no plugin system.** `crates/jcode-base/src/hooks.rs` (1,374 lines) is the entire extension
  mechanism: named lifecycle events dispatched to *subprocesses* via environment variables and stdin,
  with four interaction modes — observer (detached, fire-and-forget), gate (`pre_tool`, exit 2 blocks or
  JSON stdout with `permissionDecision`/`updatedInput`/`additionalContext`), transformer
  (`pre_tool_transform`, replaces tool input JSON), and feedback (`post_tool_feedback`, appends text to a
  tool result). All eight keys are fixed command strings or arrays of them.
- **Pre-generation injection already exists and is public.**
  `crates/jcode-base/src/memory/pending.rs` exposes `set_pending_memory(session_id, prompt, count)`; the
  turn loop picks it up and appends it as a **trailing user message to preserve the cache prefix**
  (`crates/jcode-app-core/src/agent/turn_loops.rs`, around the "Inject memory as a user message at the
  end" block), then records the injection.
- **That path is owned by the memory feature.** `build_memory_prompt_nonblocking_shared`
  (`agent/prompting.rs`) returns `None` when `memory_enabled` is false, and the surrounding module
  carries duplicate-suppression state (`MEMORY_REPEAT_SUPPRESSION_SECS`, `MEMORY_SET_OVERLAP_SUPPRESSION_RATIO`,
  injected-ID tracking) that belongs to memory, not to a general plugin channel.
- **Tools are already dynamically registrable.** `crates/jcode-base/src/mcp/tool.rs` pushes
  `Arc::new(mcp_tool) as Arc<dyn Tool>` into the tool list, so a `dyn Tool` abstraction and a dynamic
  registration path exist and are exercised by MCP.
- **Two request assembly paths.** `agent/turn_loops.rs` (1,373 lines) and `agent/turn_streaming_mpsc.rs`
  (2,110 lines) both assemble requests and both contain memory-injection logic. Anything added to one must
  be added to the other or they diverge.
- **Performance is the product.** jcode's differentiation is RAM per session (~3.3 MB marginal) and time
  to first input (~19 ms). Any default-path cost is a product regression, not an implementation detail.
- **jcode is an upstream project, and this checkout is also a platform foundation.** Keeping the diff
  additive and reviewable matters both for upstreaming and for rebasing onto later releases.

## Goals / Non-Goals

**Goals:**

- One declared, versioned, bidirectional extension channel that covers observation, context contribution,
  tool registration, and gating — replacing the current four unrelated surfaces for anything stateful.
- Zero cost and zero new failure modes when no plugin is configured.
- A plugin author can develop and test without starting a session, in any language.
- jcode stays upstreamable: additive modules, no rewrites of the existing hook path.

**Non-Goals:**

- WASM or in-process Rust plugin ABIs (see D1).
- A plugin marketplace, UI, or version manager.
- Sandboxing beyond process isolation: a plugin runs with the user's privileges, as hooks already do.
- Replacing lifecycle hooks. They remain the zero-install path and must keep working identically.
- Enforcing resource limits per plugin (CPU/memory) in v1.
- The brainiac memory plugin itself. This change delivers the host and the reference plugin; a memory
  plugin is a follow-on that consumes it.

## Decisions

### D1. Out-of-process plugins over line-delimited JSON on stdio

A plugin is a child process; jcode writes requests and reads responses as one JSON object per line.

Alternatives rejected: **in-process Rust trait objects** (fastest, but requires rebuilding jcode for every
plugin, no isolation, and an unstable ABI across a 90-crate workspace); **WASM/WASI** (good sandboxing and
language coverage, but adds a runtime dependency, an ABI and host-function design, and cannot do what the
first consumer needs — talking to a local HTTP service with its own lifecycle); **extending the existing
hook protocol only** (one process per event, no state, no bidirectional channel, and no way to answer a
request synchronously at moderate cost).

Stdio is chosen over a unix socket because it needs no filesystem coordination, no path collision between
sessions, and inherits the child's lifetime from jcode's.

### D2. Context contribution gets its own channel, not the memory queue

A new `PluginContext` queue sits beside the memory queue, and the turn loop appends plugin contributions
as additional trailing messages. It deliberately does **not** reuse `set_pending_memory`.

Rationale, from the code: that path is gated by `memory_enabled`, so a plugin would silently stop working
when an operator turned memory off — an unrelated flag disabling an unrelated feature. It also carries
memory's duplicate-suppression heuristics, which would suppress a plugin's context when it resembles
recent memory, and its injected-ID bookkeeping, which is meaningless for a plugin. Sharing the mechanism
(trailing message, cache-safe) while separating the state is the distinction that matters.

Ordering is fixed: contributions are appended after history, after memory, in configured plugin order, so
the sequence is deterministic and documented.

### D3. The event vocabulary is reused, not reinvented

The plugin protocol carries the same events jcode already dispatches to hooks (`session_start`,
`session_end`, `turn_start`, `turn_end`, `pre_tool`, `post_tool`) with the same payload fields. A new
event list would mean two vocabularies drifting apart, and hook payloads are already the documented
contract in `docs/HOOKS.md`.

The plugin protocol is not a transport change to hooks: hooks keep their env-var/subprocess-per-event
model, and a configured plugin coexists with a configured hook on the same event.

### D4. Failure degrades, and the fallback is stated

Every extension point has a per-point timeout and a documented fallback: absent contribution, absent
decision, or the configured gate fallback. The default for gates mirrors hooks' existing fail-open
behaviour (a broken policy script must not brick every session), but it is configurable to fail-closed
and the applied fallback is reported in diagnostics, because an invisible fallback is indistinguishable
from a plugin that silently stopped working.

A plugin that dies is not restarted within the session; it is marked unusable, the session continues, and
the failure surfaces. Restart loops are how a slow plugin becomes a hung session.

### D5. Tools register through the existing dynamic path

Registered tools become `Arc<dyn Tool>` entries through the same route MCP uses, prefixed with the plugin
name so they are distinguishable and cannot shadow a built-in, and validated by the same JSON-schema
checks a hand-written tool passes.

Alternative rejected: a parallel tool registry for plugins, which would need its own validation, naming,
and disable logic and would drift from the built-in path.

### D6. The handshake is versioned, and capabilities are granted by configuration

Startup exchanges a version and a capability list in both directions. jcode refuses a version it does not
speak, and exercises only capabilities it has both granted (via config) and the plugin has declared. A
plugin cannot widen its own grant at runtime.

Rationale: the alternative — trust whatever the plugin asks for — makes a plugin's code the source of its
own permissions, and it makes "why did this plugin see my tool inputs?" unanswerable.

### D7. One process per session, started lazily, killed with the session

The host starts on first use in a session and exits when the session ends, with a hard kill on teardown
so a wedged plugin cannot outlive its session. Starting lazily keeps sessions with no plugin activity
free of a process.

Alternative rejected: one process per jcode instance shared across sessions, which improves startup at the
cost of cross-session isolation and of leaking one session's plugin failure into another.

### D8. The default path gains no latency

With no plugin configured, the turn loop must take the same path it takes today: no new allocation of a
contribution list, no new await, no new diagnostic. This is a review criterion, not a nicety, and it is
measured in the verification tasks against a baseline captured before the change.

### D9. Both turn loops get the change through one helper

Request assembly is duplicated across `turn_loops.rs` and `turn_streaming_mpsc.rs`. The contribution
append is added as a single shared function called from both, so a future third path has one place to
call rather than a copy to notice. The risk stays: if a new request path is added without calling the
helper, plugins silently do not contribute there — recorded as a risk and covered by a test that asserts
both paths call it.

## Risks / Trade-offs

- **Performance is the product, and this adds a process per session** → lazy start, one process per
  session, per-point timeouts, and an explicit no-plugin baseline measurement as a merge gate.
- **Two turn loops drift** → one shared helper, plus a test that asserts both request paths produce a
  contribution when a plugin is configured.
- **Suppressed context looks like a working plugin** → every fallback and timeout is reported; the plugin
  test command exists so an operator can tell "not running" from "running and silent".
- **A plugin with the user's privileges is a trust decision** → capability grants in configuration,
  explicit untrusted labelling of contributed text, and documentation that process isolation is not a
  sandbox.
- **Upstream divergence and rebase cost** → additive modules beside `hooks.rs`, no rewrite of the hook
  path, a config table rather than changed existing keys, and no edits to `hooks.rs` itself.
- **Protocol churn as the first consumer teaches us things** → the version is negotiated per connection,
  so a v2 can be added without breaking v1 plugins.
- **A wedged plugin stalls a turn** → hard timeout per extension point, and a hard kill on session
  teardown.

## Migration Plan

Additive. With no `[[plugins]]` entries, configuration is unchanged and behaviour is identical; existing
`[hooks]` keys keep their semantics and can run alongside a plugin on the same event.

Rollback is removing the plugin entries; no persisted format changes, so a session recorded with plugins
replays without them.

## Open Questions

- Whether the reference plugin should ship as a second binary in the workspace or as a script in
  `scripts/`. Deferrable: either exercises the same protocol.
- Whether contributions should be persisted for replay fidelity. Deferrable: it changes a storage
  decision, not the protocol.
- Whether per-plugin memory/CPU limits are worth adding later. Deferrable: v1 relies on process isolation
  and the user's own trust decision.
- Whether the plugin host should also expose the harness-API server path (`system_reminder`) so API-driven
  sessions get the same contributions. Deferrable: it is the same channel, reached differently.
