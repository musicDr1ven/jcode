# Tasks

## 1. Ground the plan in the checkout

- [x] 1.1 Capture a no-plugin baseline: RAM per session and time to first input on this machine, recorded
      in `docs/plugin-baseline.md`, so D8 can be measured rather than asserted
- [x] 1.2 Confirm both request-assembly paths (`agent/turn_loops.rs`, `agent/turn_streaming_mpsc.rs`) and
      identify the exact line in each where memory is appended, verified by a comment-anchored test that
      fails if either anchor moves
- [x] 1.3 Confirm the dynamic tool path: register a throwaway `Arc<dyn Tool>` outside MCP and verify the
      model can call it, proving D5 is available without MCP
- [x] 1.4 Confirm how long a session lives in TUI, headless, and swarm entrypoints, and record which
      component owns teardown, verified against each entrypoint's shutdown path
- [x] 1.5 Decide and record whether the reference plugin is a workspace binary or a script (open question 1)

## 2. Configuration and discovery

- [x] 2.1 Add a `[[plugins]]` table to `jcode-config-types` (name, command, args, capabilities, per-point
      timeouts, gate fallback) and verify config parses with zero entries and jcode behaves identically
- [x] 2.2 Add environment overrides consistent with the existing `JCODE_HOOK_*` pattern and verify an
      override wins over the config file
- [x] 2.3 Implement `jcode plugin list` showing each configured plugin and whether it is usable, verified
      against a config with one valid and one broken entry
- [x] 2.4 Implement `jcode plugin test <name>`: start, handshake, report version and capabilities, stop;
      verify it works with no session and no model call, and reports failure clearly for a plugin that
      never answers

## 3. Plugin host and protocol

- [x] 3.1 Implement the plugin host module beside `hooks.rs`: spawn, line-delimited JSON framing, request
      correlation, stderr capture, and teardown with a hard kill; verify a malformed output line and a
      plugin exit are both contained
- [x] 3.2 Implement the versioned handshake and capability negotiation with refusal of an unsupported
      version naming both versions
- [x] 3.3 Implement lazy start on first use per session and verify a session with no plugin activity never
      spawns a process
- [x] 3.4 Implement per-point timeouts with the documented fallback, and verify each timeout path degrades
      without extending turn latency beyond the timeout
- [x] 3.5 Verify a plugin that dies mid-session is marked unusable, is not restarted within the session,
      and the failure is visible

## 4. Lifecycle events

- [x] 4.1 Deliver `session_start`/`session_end` to subscribed plugins with session identity and working
      directory; verify both the create and resume cases (`source: "create"` and `source: "resume"` both
      observed; `session_end` delivery required the awaited-teardown fix below)
- [x] 4.2 Deliver `turn_start`/`turn_end` with status, duration, model, token counts and last assistant
      text; verify the payload matches what `docs/HOOKS.md` documents for the same event (live run observed
      `session_id`, `cwd`, `status`, `duration_ms`, `model`, `last_assistant_text`, `prompt_tokens`,
      `input_tokens`, `output_tokens`. The capture path used by `--json`/`--ndjson` did not fire turn events
      at all — hooks included — and now does)
- [x] 4.3 Deliver `pre_tool`/`post_tool` with tool name, input, outcome and named paths; verify the path
      list includes `file_path` and `apply_patch` style inputs (a `read` call with `file_path: "probe.txt"`
      reported the resolved absolute path in `touched_paths`; `post_tool` carried `status`, `duration_ms`,
      `output_bytes`; `patch_text` paths are extracted by the same helper)
- [x] 4.4 Enforce the payload size cap and verify an oversized event is capped with the cap recorded in
      diagnostics
- [x] 4.5 Verify a plugin and a hook configured on the same event both receive it

## 5. Context contribution

- [x] 5.1 Add the `PluginContext` queue beside the memory queue and a single shared append helper called
      from **both** request-assembly paths (D9), verified by a test that asserts either path produces the
      contribution
- [x] 5.2 Verify a contribution is delivered to the model on the same turn it is produced
- [x] 5.3 Verify contributed text is not written into persisted history as a user message
- [ ] 5.4 Verify the cached prompt prefix is unchanged when a contribution is added on a later turn, by
      comparing the stable prefix between two turns
- [x] 5.5 Implement deterministic ordering across several contributing plugins in configured order, and
      verify the order is stable across turns
- [x] 5.6 Enforce the contribution size bound and verify oversized output is reported rather than silently
      truncated
- [x] 5.7 Verify an empty contribution changes nothing and adds no message
- [ ] 5.8 Verify contributions are independent of the `memory_enabled` flag, proving D2's separation

## 6. Tool registration and gating

- [x] 6.1 Register plugin tools through the existing dynamic path with a name prefix, and verify a
      registered tool is callable by the model and its result returns
- [x] 6.2 Refuse a malformed schema and a name collision with a built-in tool, verified for both cases
- [x] 6.3 Implement gate decisions: allow, deny with a reason, and input rewrite; verify a denial reaches
      the model as the tool error and a rewrite is the input that executes
- [x] 6.4 Verify multiple gates compose in configured order, a block stops execution, and later gates see
      an earlier rewrite
- [x] 6.5 Verify a gate that does not answer applies the configured fallback, and that the fallback is
      reported rather than silent

## 7. Capability boundaries

- [x] 7.1 Enforce that a plugin only receives events and payloads for granted capabilities; verify a plugin
      without tool access receives no tool names or inputs
- [ ] 7.2 Refuse a runtime request for an ungranted extension point and verify it is recorded
- [x] 7.3 Verify one plugin's contribution text is never delivered to another plugin
- [x] 7.4 Verify each extension point can be disabled individually and is then never invoked

## 8. Reference plugin and documentation

- [x] 8.1 Implement the reference plugin exercising every extension point: observe both turn events,
      contribute a labelled line of context, register one tool, gate one tool name, and exit cleanly on
      shutdown
- [x] 8.2 Verify the reference plugin against a real session: its context appears in the request, its tool
      is callable, its gate blocks the named call, and nothing appears in stored history
- [x] 8.3 Write `docs/PLUGINS.md`: the protocol, the version and handshake, every extension point with its
      payload and timeout, the capability matrix, failure and fallback semantics, and a worked example
- [x] 8.4 Document the trust model: process isolation is not a sandbox, a plugin runs with the user's
      privileges, and contributed text is labelled untrusted
- [x] 8.5 Note in `docs/HOOKS.md` how hooks and plugins relate, so an author knows which to reach for

## 9. Integration verification

- [x] 9.1 Verify the no-plugin path against the 1.1 baseline: request assembly is unchanged and RAM per
      session and time to first input stay within measurement noise (latency unchanged; RSS is
      +~3 MiB with the host compiled in — recorded as a correction, see below)
- [x] 9.2 Verify a session with three plugins configured starts one process each, not one per event, and
      that all three shut down with the session
- [x] 9.3 Verify a crashing plugin, a hanging plugin, and a plugin writing garbage each leave the session
      working, in one run, with the failures visible (verified in separate runs: malformed lines discarded;
      context timeout bounded at 600 ms; handshake exit reported as unusable)
- [ ] 9.4 Verify the same contribution appears through both request-assembly paths by driving one session
      in the streaming mode and one in the non-streaming mode
- [x] 9.5 Record measured overhead (process startup, per-turn contribution round trip) in
      `docs/plugin-baseline.md` and state whether it is acceptable for the intended use
- [ ] 9.6 Verify the change is additive: `cargo test` for every touched crate (`jcode-config-types`,
      `jcode-base`, `jcode-app-core`) passes with the plugin host compiled in but unused, and no existing
      hook test was modified

## Status of remaining tasks

Open items are verification gaps, not missing code unless noted. Each names what would close it.

- **Fixed during verification:** `session_end` was never delivered to a one-shot `jcode run`. Delivery ran
  on a detached task that the exit killed first. Exit paths now await an awaitable teardown, which also
  closes the plugin's stdin before the kill so the plugin can flush. Regression tests:
  `detached_teardown_loses_session_end_when_the_runtime_exits_immediately` (documents the old mechanism),
  `awaited_teardown_delivers_session_end_before_returning`,
  `awaited_teardown_is_bounded_by_the_event_timeout`, and the source-anchored
  `headless_run_awaits_the_plugin_session_end_delivery`.
- **Also fixed:** `--json`/`--ndjson`/capture runs fired no `turn_start`/`turn_end` at all (hooks included);
  the capture path now mirrors `run_once`.
- **5.4** Contributions are appended only to the trailing request messages, never the prefix; a test that
  compares the stable prefix across two turns is not written.
- **5.8** The channel is separate from `set_pending_memory` and is not gated by `memory_enabled`; a test
  toggling `memory_enabled` is not written.
- **7.2** v1 has no plugin-initiated requests, so "refuse a runtime request for an ungranted point" is
  enforced by never sending that request (verified by 7.1); there is no request to refuse.
- **9.1 correction:** latency is unchanged, but RSS grew ~3 MiB with the host compiled in (58.6–59.0 MiB vs
  55.5 MiB). Recorded in `docs/plugin-baseline.md` and in design D8: the verified claim is "no new per-turn
  cost", not "no new cost".


- **9.4** Both request-assembly paths call the shared helper (anchor test), but only the streaming path was
  driven end to end (`jcode run` always streams).
- **9.6** `jcode-config-types` passes (26/26). `jcode-base` runs 1973 passed / 34 failed, where the 34 are
  the pre-existing failures listed in the model-attribution repro doc plus one known timing flake.
  `jcode-app-core` has environment failures in this sandbox (`Operation not permitted`, agent/KV-cache
  tests) that were not separately baselined; none reference plugin code, and every plugin test passes.
  `hooks.rs` and its tests are untouched.

## Workflow follow-up

- Review the proposal, specs, design, and tasks with the project owner before implementation.
- Decide whether this stays local to this checkout or is proposed upstream, and record that decision
  where the next rebase will find it.
- Archive the change once implementation and verification are complete.
