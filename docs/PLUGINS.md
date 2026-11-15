# Plugins

Plugins are jcode's stateful, bidirectional extension channel. A plugin is a
**long-lived child process** that speaks **line-delimited JSON on stdio** and can
observe a session, contribute context before the model runs, add tools, and gate
or rewrite tool calls.

For the stateless, zero-install path, keep using [lifecycle hooks](HOOKS.md).
Hooks and plugins coexist: a hook and a plugin configured for the same event both
receive it.

> **Trust model.** Process isolation is **not** a sandbox. A plugin runs with
> your privileges and can read and write anything you can. Only install plugins
> you trust, and grant only the capabilities you need. Text a plugin contributes
> is labelled as untrusted input to the model.

## Configuration

```toml
[[plugins]]
name = "reference"
command = "python3"
args = ["scripts/reference_plugin.py"]
capabilities = ["observe", "tool_events", "context", "tools", "gate"]

# Optional, per plugin:
gate_fallback = "allow"            # or "deny" (default: "allow", matching hooks)
[plugins.timeouts]
handshake_ms = 3000
event_ms = 1000
context_ms = 2000
tool_gate_ms = 2000
tool_call_ms = 30000
```

With no `[[plugins]]` entry jcode starts no process and request assembly is
unchanged.

Environment overrides, matching the `JCODE_HOOK_*` convention:

| Variable | Effect |
| --- | --- |
| `JCODE_PLUGINS` | Replaces the whole plugin list with a TOML `[[plugins]]` table array. Empty disables all plugins. |
| `JCODE_PLUGIN_<NAME>_COMMAND` | Overrides the command for the plugin named `<NAME>` (uppercased, non-alphanumerics become `_`). Empty disables that entry. |

A plugin's process is started **lazily on first use in a session** and is killed
when the session ends. One process serves every event in that session; nothing
is started when a session has no plugin activity.

## Protocol (v1)

One JSON object per line. jcode writes to the plugin's stdin and reads its
stdout. **stderr is diagnostics, never protocol**; jcode logs it.

### Handshake

jcode opens with:

```json
{"protocol":1,"type":"hello","id":1,"protocol_versions":[1],
 "session_id":"session_abc","cwd":"/work/repo",
 "granted_capabilities":["observe","context"]}
```

The plugin answers:

```json
{"type":"hello","protocol":1,"name":"reference","version":"1.0.0",
 "capabilities":["observe","context","tools"]}
```

A plugin that reports a version jcode does not speak is refused, with a
diagnostic naming both versions. The effective capabilities are the
**intersection** of what configuration grants and what the plugin declares; a
plugin can never widen its grant at runtime.

### Requests

jcode → plugin, correlated by `id`:

```json
{"protocol":1,"type":"request","id":7,"method":"context","params":{...}}
```

The plugin answers:

```json
{"type":"response","id":7,"ok":true,"result":{...}}
{"type":"response","id":7,"ok":false,"error":"why it failed"}
```

Non-JSON lines, unknown message types, and stale response ids are reported and
discarded without breaking the session.

### Events

Fire-and-forget notifications (no response):

```json
{"protocol":1,"type":"event","event":"turn_start","payload":{...}}
```

## Capabilities and extension points

| Capability | Grants | Delivered as |
| --- | --- | --- |
| `observe` | `session_start`, `session_end`, `turn_start`, `turn_end` | events |
| `tool_events` | `pre_tool`, `post_tool` (carry tool names and inputs) | events |
| `context` | contribute text before generation | request `context` |
| `tools` | register callable tools | request `tools/list`, `tools/call` |
| `gate` | permit, deny, or rewrite a tool call | request `gate` |

A capability left out of `capabilities` is never exercised. A plugin without
`tool_events` receives no tool names or inputs; a plugin without `gate` is never
asked to decide a call.

### Lifecycle event payloads

Payload fields mirror the [hook fields](HOOKS.md) under lowercase JSON names.

- `session_start` / `session_end`: `session_id`, `cwd`, `source`, `model`.
- `turn_start`: `session_id`, `cwd`, `model`, `source`.
- `turn_end`: `session_id`, `cwd`, `status` (`ok`/`error`), `duration_ms`,
  `model`, `last_assistant_text`, and when available `prompt_tokens`,
  `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`,
  `error`.
- `pre_tool`: `session_id`, `cwd`, `tool_name`, `tool_call_id`, `tool_input`,
  `touched_paths`.
- `post_tool`: the same plus `status`, `duration_ms`, `output_bytes` or `error`.

An event payload above 64 KiB is replaced with a small envelope
(`{"truncated":true,"original_bytes":…,"cap_bytes":65536}`) and the cap is
recorded in diagnostics.

### Context contribution

Request:

```json
{"type":"request","id":2,"method":"context",
 "params":{"session_id":"…","cwd":"…","plugin":"reference","max_bytes":16384}}
```

Response:

```json
{"type":"response","id":2,"ok":true,"result":{"text":"…"}}
```

Contributions are appended **after history and after memory**, in configured
plugin order, as trailing messages that are labelled as plugin-supplied and
never written into the stored transcript. The stable prompt prefix is unchanged,
so the provider cache is preserved. An empty contribution adds nothing. A
contribution above the bound is truncated with a visible marker and recorded in
diagnostics.

### Tools

`tools/list` returns:

```json
{"tools":[{"name":"echo","description":"Echo text back.",
           "input_schema":{"type":"object","properties":{"text":{"type":"string"}}}}]}
```

Validation: a tool needs a non-empty name, a description, and an object input
schema. A malformed schema, a missing field, or a name that collides with a
built-in or another tool is refused and reported. The model-facing name is
`<plugin>__<tool>` (for example `reference__echo`), so plugin tools can never
shadow a built-in.

`tools/call` receives the plugin's own tool name:

```json
{"type":"request","id":9,"method":"tools/call",
 "params":{"name":"echo","input":{"text":"hi"}}}
```

and answers with `{"result":{"output":"…"}}`.

### Gates

Request:

```json
{"type":"request","id":4,"method":"gate",
 "params":{"session_id":"…","tool_name":"bash","input":{"command":"…"}}}
```

Answer with exactly one of:

```json
{"result":{"decision":"allow"}}
{"result":{"decision":"deny","reason":"why the model should see this"}}
{"result":{"decision":"rewrite","input":{"command":"echo safe"}}}
```

Gates run **after** `pre_tool` hooks and before validation/execution, in
configured plugin order. A rewrite is what later gates see and what executes;
a denial is returned to the model as the tool call's error. A gate that times
out or answers unusably applies the plugin's `gate_fallback` (`allow` by
default, configurable to `deny`), and the applied fallback is reported in
diagnostics.

## Timeouts and failure

Each extension point has its own timeout (table above). A missed deadline
degrades that point, never blocks the turn indefinitely:

- context: the turn proceeds without that contribution, and the timeout is logged;
- gate: the configured fallback decides;
- event: the write is dropped and logged.

A plugin that exits, crashes, or writes garbage is marked unusable for the rest
of the session. It is **not restarted**, its extension points behave as if it
were never configured, and the failure is logged. A plugin that fails to start
is skipped with its startup error in the log.

## Operator commands

```bash
jcode plugin list            # each configured plugin, usable or not
jcode plugin test reference  # start, handshake, report version/capabilities, stop
```

Both run without a session or a model call. `jcode plugin test` exits non-zero
when the plugin fails.

## Reference plugin

[`scripts/reference_plugin.py`](../scripts/reference_plugin.py) exercises every
extension point: it observes events on stderr, contributes one line of context,
registers `reference__echo`, and gates `bash` (denying `rm -rf`, rewriting the
command `rewrite me`). It is the worked example for this document.

## Worked example

```toml
[[plugins]]
name = "reference"
command = "python3"
args = ["scripts/reference_plugin.py"]
capabilities = ["observe", "tool_events", "context", "tools", "gate"]
```

Then:

```bash
jcode plugin test reference
jcode -p openai-compatible run "Use the reference__echo tool to echo 'hello'"
```

The request carries the reference plugin's context line, `reference__echo` is
callable, and a `bash` call whose command is `rm -rf …` is refused with the
plugin's reason.
