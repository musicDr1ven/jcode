# Tasks

## 1. Ground the plan in the checkout

- [ ] 1.1 Capture a no-plugin baseline: RAM per session and time to first input on this machine, recorded
      in `docs/plugin-baseline.md`, so D8 can be measured rather than asserted
- [ ] 1.2 Confirm both request-assembly paths (`agent/turn_loops.rs`, `agent/turn_streaming_mpsc.rs`) and
      identify the exact line in each where memory is appended, verified by a comment-anchored test that
      fails if either anchor moves
- [ ] 1.3 Confirm the dynamic tool path: register a throwaway `Arc<dyn Tool>` outside MCP and verify the
      model can call it, proving D5 is available without MCP
- [ ] 1.4 Confirm how long a session lives in TUI, headless, and swarm entrypoints, and record which
      component owns teardown, verified against each entrypoint's shutdown path
- [ ] 1.5 Decide and record whether the reference plugin is a workspace binary or a script (open question 1)

## 2. Configuration and discovery

- [ ] 2.1 Add a `[[plugins]]` table to `jcode-config-types` (name, command, args, capabilities, per-point
      timeouts, gate fallback) and verify config parses with zero entries and jcode behaves identically
- [ ] 2.2 Add environment overrides consistent with the existing `JCODE_HOOK_*` pattern and verify an
      override wins over the config file
- [ ] 2.3 Implement `jcode plugin list` showing each configured plugin and whether it is usable, verified
      against a config with one valid and one broken entry
- [ ] 2.4 Implement `jcode plugin test <name>`: start, handshake, report version and capabilities, stop;
      verify it works with no session and no model call, and reports failure clearly for a plugin that
      never answers

## 3. Plugin host and protocol

- [ ] 3.1 Implement the plugin host module beside `hooks.rs`: spawn, line-delimited JSON framing, request
      correlation, stderr capture, and teardown with a hard kill; verify a malformed output line and a
      plugin exit are both contained
- [ ] 3.2 Implement the versioned handshake and capability negotiation with refusal of an unsupported
      version naming both versions
- [ ] 3.3 Implement lazy start on first use per session and verify a session with no plugin activity never
      spawns a process
- [ ] 3.4 Implement per-point timeouts with the documented fallback, and verify each timeout path degrades
      without extending turn latency beyond the timeout
- [ ] 3.5 Verify a plugin that dies mid-session is marked unusable, is not restarted within the session,
      and the failure is visible

## 4. Lifecycle events

- [ ] 4.1 Deliver `session_start`/`session_end` to subscribed plugins with session identity and working
      directory; verify both the create and resume cases
- [ ] 4.2 Deliver `turn_start`/`turn_end` with status, duration, model, token counts and last assistant
      text; verify the payload matches what `docs/HOOKS.md` documents for the same event
- [ ] 4.3 Deliver `pre_tool`/`post_tool` with tool name, input, outcome and named paths; verify the path
      list includes `file_path` and `apply_patch` style inputs
- [ ] 4.4 Enforce the payload size cap and verify an oversized event is capped with the cap recorded in
      diagnostics
- [ ] 4.5 Verify a plugin and a hook configured on the same event both receive it

## 5. Context contribution

- [ ] 5.1 Add the `PluginContext` queue beside the memory queue and a single shared append helper called
      from **both** request-assembly paths (D9), verified by a test that asserts either path produces the
      contribution
- [ ] 5.2 Verify a contribution is delivered to the model on the same turn it is produced
- [ ] 5.3 Verify contributed text is not written into persisted history as a user message
- [ ] 5.4 Verify the cached prompt prefix is unchanged when a contribution is added on a later turn, by
      comparing the stable prefix between two turns
- [ ] 5.5 Implement deterministic ordering across several contributing plugins in configured order, and
      verify the order is stable across turns
- [ ] 5.6 Enforce the contribution size bound and verify oversized output is reported rather than silently
      truncated
- [ ] 5.7 Verify an empty contribution changes nothing and adds no message
- [ ] 5.8 Verify contributions are independent of the `memory_enabled` flag, proving D2's separation

## 6. Tool registration and gating

- [ ] 6.1 Register plugin tools through the existing dynamic path with a name prefix, and verify a
      registered tool is callable by the model and its result returns
- [ ] 6.2 Refuse a malformed schema and a name collision with a built-in tool, verified for both cases
- [ ] 6.3 Implement gate decisions: allow, deny with a reason, and input rewrite; verify a denial reaches
      the model as the tool error and a rewrite is the input that executes
- [ ] 6.4 Verify multiple gates compose in configured order, a block stops execution, and later gates see
      an earlier rewrite
- [ ] 6.5 Verify a gate that does not answer applies the configured fallback, and that the fallback is
      reported rather than silent

## 7. Capability boundaries

- [ ] 7.1 Enforce that a plugin only receives events and payloads for granted capabilities; verify a plugin
      without tool access receives no tool names or inputs
- [ ] 7.2 Refuse a runtime request for an ungranted extension point and verify it is recorded
- [ ] 7.3 Verify one plugin's contribution text is never delivered to another plugin
- [ ] 7.4 Verify each extension point can be disabled individually and is then never invoked

## 8. Reference plugin and documentation

- [ ] 8.1 Implement the reference plugin exercising every extension point: observe both turn events,
      contribute a labelled line of context, register one tool, gate one tool name, and exit cleanly on
      shutdown
- [ ] 8.2 Verify the reference plugin against a real session: its context appears in the request, its tool
      is callable, its gate blocks the named call, and nothing appears in stored history
- [ ] 8.3 Write `docs/PLUGINS.md`: the protocol, the version and handshake, every extension point with its
      payload and timeout, the capability matrix, failure and fallback semantics, and a worked example
- [ ] 8.4 Document the trust model: process isolation is not a sandbox, a plugin runs with the user's
      privileges, and contributed text is labelled untrusted
- [ ] 8.5 Note in `docs/HOOKS.md` how hooks and plugins relate, so an author knows which to reach for

## 9. Integration verification

- [ ] 9.1 Verify the no-plugin path against the 1.1 baseline: request assembly is unchanged and RAM per
      session and time to first input stay within measurement noise
- [ ] 9.2 Verify a session with three plugins configured starts one process each, not one per event, and
      that all three shut down with the session
- [ ] 9.3 Verify a crashing plugin, a hanging plugin, and a plugin writing garbage each leave the session
      working, in one run, with the failures visible
- [ ] 9.4 Verify the same contribution appears through both request-assembly paths by driving one session
      in the streaming mode and one in the non-streaming mode
- [ ] 9.5 Record measured overhead (process startup, per-turn contribution round trip) in
      `docs/plugin-baseline.md` and state whether it is acceptable for the intended use
- [ ] 9.6 Verify the change is additive: `cargo test` for every touched crate (`jcode-config-types`,
      `jcode-base`, `jcode-app-core`) passes with the plugin host compiled in but unused, and no existing
      hook test was modified

## Workflow follow-up

- Review the proposal, specs, design, and tasks with the project owner before implementation.
- Decide whether this stays local to this checkout or is proposed upstream, and record that decision
  where the next rebase will find it.
- Archive the change once implementation and verification are complete.
