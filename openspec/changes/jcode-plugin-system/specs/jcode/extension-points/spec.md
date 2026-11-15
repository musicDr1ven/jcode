# Spec Delta

## Purpose

Defines what a plugin can observe and influence inside a jcode session: the lifecycle events delivered
to it, the context it can place in front of the model before generation, the tools it can add, and the
tool calls it can gate or rewrite — together with the ordering, visibility, and trust rules that make
those points safe to rely on.

## ADDED Requirements

### Requirement: Lifecycle events reach plugins that subscribed to them

A plugin SHALL receive the lifecycle events it declared interest in: session start and end, turn start
and end, and tool calls before and after execution. Each event MUST carry the session identity, the
working directory, and the event-specific fields jcode already knows for that moment.

#### Scenario: Turn end carries the outcome

- **WHEN** a turn completes
- **THEN** a subscribed plugin receives the status, duration, model, token counts, and the last
  assistant text

#### Scenario: Tool events carry the call

- **WHEN** a tool executes
- **THEN** a subscribed plugin receives the tool name, its input, the outcome, and the paths the call
  named

#### Scenario: Delivery is bounded

- **WHEN** an event's payload is large
- **THEN** it is capped at a documented size rather than passed unbounded to the plugin

### Requirement: A plugin can contribute context before generation

A plugin SHALL be able to supply text that jcode places in the request to the model for the current turn.
The contribution MUST be labelled as plugin-supplied, MUST be delivered as a trailing addition rather
than a rewrite of earlier content, and MUST be attributable to the plugin that produced it.

#### Scenario: Contribution precedes the model call

- **WHEN** a plugin contributes context for a turn
- **THEN** the model request for that turn contains it

#### Scenario: Contribution is attributed

- **WHEN** context appears in a request
- **THEN** its source plugin is identifiable in the assembled request and in diagnostics

#### Scenario: Empty contribution is not an error

- **WHEN** a plugin returns no context
- **THEN** the turn proceeds unchanged and adds nothing

### Requirement: Context contributions are ordered and bounded

When several plugins contribute context in one turn, their order SHALL be deterministic and documented,
and each contribution MUST respect a size bound. A contribution exceeding the bound MUST be reported
rather than silently truncated without trace.

#### Scenario: Deterministic order

- **WHEN** two plugins both contribute on the same turn
- **THEN** their contributions appear in a stable order across turns

#### Scenario: Oversized contribution

- **WHEN** a plugin's contribution exceeds its bound
- **THEN** the outcome (truncated or rejected) is recorded and the plugin is told

### Requirement: A plugin can register tools

A plugin SHALL be able to add tools that the model can call. A registered tool MUST declare a name, a
description, and an argument schema before it becomes callable, and MUST be distinguishable from
built-in tools in its naming and in the tool list.

#### Scenario: Registered tool is callable

- **WHEN** a plugin registers a tool with a valid schema
- **THEN** the model can call it and its result returns to the model

#### Scenario: Invalid schema is refused

- **WHEN** a plugin registers a tool whose schema is malformed
- **THEN** the tool is not offered to the model and the reason is reported

#### Scenario: Name collisions are refused

- **WHEN** a plugin registers a name that collides with a built-in or existing tool
- **THEN** registration is refused rather than shadowing the built-in

### Requirement: A plugin can gate or rewrite a tool call

Before a tool executes, a plugin SHALL be able to permit, deny with a reason, or replace the call's
input. A denial MUST return its reason to the model so it can adapt, and a rewrite MUST be the input that
is validated and executed.

#### Scenario: Denial explains itself

- **WHEN** a plugin denies a tool call with a reason
- **THEN** the call does not execute and the model receives that reason

#### Scenario: Rewrite is what runs

- **WHEN** a plugin replaces a call's input
- **THEN** the replaced input is validated and executed, and later gates see the replacement

#### Scenario: Several gates compose

- **WHEN** multiple plugins gate the same call
- **THEN** they run in a deterministic order, a block stops execution, and rewrites are visible to
  subsequent gates

### Requirement: Extension points declare their cost and can be disabled individually

Each extension point SHALL be individually disableable per plugin, and a disabled extension point MUST
not be invoked. Configuration MUST let an operator keep a plugin's cheap observation without paying for
an expensive per-tool contribution.

#### Scenario: Per-point disable

- **WHEN** a plugin's context contribution is disabled in configuration
- **THEN** its other declared extension points still work and it is never asked for context

#### Scenario: Hot paths stay cheap

- **WHEN** a plugin subscribes only to session events
- **THEN** nothing is asked of it during the tool-call path

### Requirement: Trust boundaries are explicit

Plugin-contributed context and tool results MUST be marked as untrusted input to the model, and a plugin
MUST NOT be able to widen its own capabilities beyond what configuration grants.

#### Scenario: Contribution is labelled as untrusted

- **WHEN** context from a plugin enters a request
- **THEN** it is presented so the model can tell it apart from instructions from the operator

#### Scenario: A plugin cannot self-grant a capability

- **WHEN** a plugin asks for an extension point its configuration does not grant
- **THEN** the request is refused and recorded

### Requirement: Plugins cannot observe what they were not granted

A plugin SHALL receive only events and payloads for the capabilities configured for it. A plugin without
tool access MUST NOT receive tool inputs, and MUST NOT receive another plugin's contributions.

#### Scenario: No tool payload without the grant

- **WHEN** a plugin is configured without tool-event access
- **THEN** it receives no tool names or inputs

#### Scenario: Contributions are not shared

- **WHEN** two plugins contribute context
- **THEN** neither receives the other's contribution text
