# Proposal

## Why

jcode is fast, and it already contains most of what a serious agent needs — which is exactly why it is
the foundation for flowbot-central. What it does not have is an extension system. Today an integration
is assembled from four unrelated surfaces: lifecycle hooks in `~/.jcode/config.toml`, an MCP server for
tools, static prompt files, and skills. Each works, none of them compose well, and the seam that matters
most — putting context in front of the model before it generates — exists in the code but is not
available to anything outside the process.

That gap has a concrete cost: a memory system like the one in this repository cannot be plugged into
jcode without either driving jcode through the harness API or accepting that recall is only whatever the
model decides to ask for. Any future capability faces the same wall, so the fix belongs in jcode once
rather than in every integration.

## What Changes

- Add a **plugin system** to jcode: plugins are declared in configuration, launched as long-lived
  processes, and speak a line-delimited JSON protocol over stdio.
- Define **extension points** a plugin can use: observe lifecycle events, contribute context before
  generation, register tools, and gate or transform tool calls.
- Reuse what jcode already proves rather than inventing parallel machinery: the existing lifecycle event
  vocabulary, the pre-generation injection path that already appends a trailing message while preserving
  the prompt cache, and the existing dynamic tool abstraction.
- Keep **existing hooks working unchanged**. Hooks remain the zero-install path; plugins are the
  stateful, bidirectional path.
- Ship a **reference plugin** that exercises every extension point, so the API is validated by something
  other than the first consumer.

## Capabilities

### New Capabilities

- `jcode/plugin-host`: how a plugin is declared, launched, talked to, and contained — configuration and
  discovery, the transport and its framing, capability negotiation, startup and shutdown, failure
  containment, and what happens when a plugin is slow, absent, or crashing.
- `jcode/extension-points`: what a plugin can actually do — the lifecycle events it observes, the context
  it can contribute before a model request, the tools it can register, the tool calls it can gate or
  rewrite, and the ordering, timeout, and trust rules at each point.

### Modified Capabilities

None. This is the first change in this repository, so no archived capability exists to modify.

## Impact

- **Target**: this repository — the jcode Rust workspace (~90 crates). This is the first change tracked
  here, so `openspec/specs/` is empty and nothing has been archived yet.
- **Touched crates**: `jcode-config-types` (a `[[plugins]]` table and capability fields),
  `jcode-base` (a plugin host module beside `hooks.rs`, and event dispatch),
  `jcode-app-core` (context contribution at both turn loops, tool registration),
  `jcode-tui`/desktop entrypoints (host startup and shutdown with the session lifecycle).
- **New frame**: a line-delimited JSON request/response protocol over a child process's stdio, versioned
  so a plugin can declare the protocol it speaks.
- **Two injection sites**: `turn_loops.rs` and `turn_streaming_mpsc.rs` both assemble a model request, so
  a context contribution must be added to both or the two paths diverge.
- **Performance**: plugins add process startup per session and a round trip per extension point. The
  design has to keep the default path (no plugins configured) free of new latency, since performance is
  jcode's differentiator.
- **Not a fork requirement**: this is additive to the existing hook dispatch, not a replacement, so
  upstream divergence stays small — worth keeping in mind because jcode is an upstream project.
