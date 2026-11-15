# Plugin baseline and cost model

Recorded before the plugin host was merged, so design D8 ("the default path gains
no latency") is measured rather than asserted.

## Environment

- macOS (arm64), release-class local build.
- Binary: `target/selfdev/jcode`, `jcode v0.99.1-dev (39a0c3ff1)` — the
  pre-plugin tree, built with `scripts/dev_cargo.sh build --profile selfdev`.
- Provider: a local fake OpenAI-compatible endpoint on `127.0.0.1` (no network,
  no model spend), model `baseline-model`.
- Command: `jcode run --no-update --socket <tmp> -p openai-compatible -m
  baseline-model "say ok"` with an isolated `JCODE_HOME`.

## Measurements

| Metric | Value | Method |
| --- | --- | --- |
| Max RSS of a headless session | **58,163,200 bytes ≈ 55.5 MiB** | `/usr/bin/time -l` around one `jcode run` |
| Time to first provider request | **702–818 ms** (two runs) | fake server timestamps the first `POST /v1/chat/completions` |
| Wall time, one turn | 1.94 s | includes 1.5 s of deliberate fake-server latency |

Caveats, recorded so later runs are comparable:

- This is a **headless** session, where the agent runs in-process with the
  client. A TUI client attached to the shared daemon has its own memory profile;
  it was not measured here.
- "Time to first provider request" is the headless proxy for time to first
  input. It includes provider initialisation, not terminal setup or first frame.
- No plugins were configured. This is the "no-plugin path" the change must keep
  unchanged (D8): same request assembly, no extra process, no extra await.

## Plugin overhead (measured with the host in place)

Measured with the host's real child-process path and a minimal shell plugin
(`crates/jcode-base/src/plugins/manager.rs`,
`process_tests::measure_plugin_overhead`):

| Phase | Measured |
| --- | --- |
| First use: spawn + handshake + first context round trip | **63 ms** |
| Steady-state context round trip (5 samples) | **2.96–4.40 ms** |

The shell plugin forks `sed` per request, so this is an upper bound for a
simple native plugin; a Python reference plugin adds interpreter startup to the
handshake. Costs are paid only by sessions that actually use a plugin: the
no-plugin path does one cached config read per event site and starts nothing.

## Session lifetime and teardown (task 1.4)

| Entrypoint | Session lifetime | Teardown owner |
| --- | --- | --- |
| TUI attached to the shared daemon | from attach/create to client disconnect or session close | the serving `Agent` (`Agent::finish` fires `session_end`; dropping the agent is the backstop) |
| Headless `jcode run` | one turn, then process exit | the client process; the in-process agent finishes and exits |
| Swarm members | each member's agent runs to completion inside the serving process | the serving `Agent` per member |

The plugin host hangs off `Agent::fire_session_lifecycle_hook("session_end", …)`:
that is where `session_ended` is called, which removes the session, delivers
`session_end` to observing plugins, and kills every process. Children are also
created with `kill_on_drop`, so process exit cannot leak a plugin. A session id
that has ended is tombstoned: a late extension-point call does not spawn a
replacement process.

## Decision: the reference plugin is a script (task 1.5)

`scripts/reference_plugin.py`, not a second workspace binary.

- It exercises the same protocol from outside the Rust workspace, which is the
  point of a reference plugin: it proves the contract is language-agnostic.
- It adds no crate, no build time, and no new dependency to the workspace.
- It can be run by hand (`python3 scripts/reference_plugin.py < hello.jsonl`),
  which makes protocol debugging cheap.

The trade-off is that automated tests need `python3`; the host's own tests use a
minimal shell plugin instead, so CI does not depend on Python.

## End-to-end verification (reference plugin, real session)

Driven with the freshly built CLI, a temporary `JCODE_HOME`, the
[reference plugin](../scripts/reference_plugin.py), and a local fake
OpenAI-compatible provider (no network, no model spend). Two scenarios, each a
`jcode run` turn where the fake provider called a tool:

| Check | Result |
| --- | --- |
| Plugin context present in the model request | ✓ |
| Context labelled as untrusted input | ✓ |
| Plugin tool advertised to the model (`reference__echo`) | ✓ |
| Tool result returns to the model | ✓ (`reference echo: hello`) |
| Gate denial reaches the model | ✓ (`rm -rf` refused with the plugin's reason) |
| Lifecycle events delivered to the plugin | ✓ (`session_start`, `turn_start`, `pre_tool`, `post_tool`, `turn_end`) |
| Hook and plugin on the same event both receive it | ✓ (hook log and plugin stderr both got the events) |
| Contributed context in persisted transcript | ✗ (correct: it is request-only) |
