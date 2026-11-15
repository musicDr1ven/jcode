# Spec Delta

## Purpose

Defines what a jcode plugin is from the outside: how one is declared and started, what protocol it
speaks, what it may claim to support, and how jcode behaves when a plugin is missing, slow, misbehaving,
or crashing. It is the contract a plugin author targets, independent of which language the plugin is
written in.

## ADDED Requirements

### Requirement: Plugins are declared in configuration and disabled by default

A plugin SHALL be declared in jcode configuration with a name, a command to run, and an optional set of
capabilities it wishes to use. With no plugins declared, jcode MUST behave exactly as before, with no
new process, no new latency, and no new failure mode.

#### Scenario: No plugins configured

- **WHEN** a session runs with no plugins declared
- **THEN** no plugin process is started and request assembly is unchanged

#### Scenario: Declaration names a command

- **WHEN** a plugin is declared
- **THEN** jcode starts it using the declared command, in the session's working directory

#### Scenario: An undeclared plugin is not started

- **WHEN** a plugin is not in configuration
- **THEN** nothing launches it and no protocol traffic occurs

### Requirement: A plugin is a long-lived process over one stdio channel

A plugin SHALL be started once per session and remain running for the session's lifetime, exchanging
line-delimited JSON messages on stdin and stdout. A plugin MUST NOT be restarted per event. Stderr MUST
be captured for diagnostics and MUST NOT be parsed as protocol.

#### Scenario: One process for many events

- **WHEN** a session produces several events
- **THEN** they are delivered to the same running process rather than starting a process per event

#### Scenario: Diagnostics are not protocol

- **WHEN** a plugin writes to stderr
- **THEN** jcode logs it and the protocol stream is unaffected

#### Scenario: Malformed output is contained

- **WHEN** a plugin writes a line that is not valid protocol JSON
- **THEN** that line is reported and discarded without breaking the session

### Requirement: The protocol is versioned and negotiated at startup

Every plugin exchange SHALL begin with a handshake in which jcode states the protocol version and the
plugin states its version and the capabilities it supports. A version jcode cannot speak MUST be refused
with a diagnostic naming both versions, rather than being used partially.

#### Scenario: Compatible handshake

- **WHEN** a plugin reports a supported protocol version
- **THEN** the session proceeds and only the capabilities it declared are used

#### Scenario: Incompatible version is refused

- **WHEN** a plugin reports a version jcode does not speak
- **THEN** the plugin is not used and the failure names both versions

#### Scenario: Undeclared capability is not exercised

- **WHEN** a plugin does not declare a capability
- **THEN** jcode never sends that capability's requests to it

### Requirement: Plugin failure never breaks the session

A plugin that fails to start, exits, times out, or returns an unusable response MUST degrade to the
behaviour jcode would have had without it. The session MUST continue, and the degradation MUST be
visible to the user rather than silent.

#### Scenario: Plugin fails to start

- **WHEN** a declared plugin's command cannot be launched
- **THEN** the session runs without it and the startup failure is reported

#### Scenario: Plugin dies mid-session

- **WHEN** a plugin exits while a session is running
- **THEN** subsequent extension points behave as if it were never configured, and the exit is reported

#### Scenario: Timeout degrades rather than hangs

- **WHEN** a plugin does not answer within its configured timeout
- **THEN** that extension point is treated as absent and the turn continues

### Requirement: Timeouts are per extension point and configurable

Each extension point SHALL have its own timeout, configurable, with a default that keeps an interactive
turn responsive. A timeout MUST NOT be treated as consent.

#### Scenario: Context contribution times out

- **WHEN** a context contribution exceeds its timeout
- **THEN** the request proceeds without that contribution and the timeout is recorded

#### Scenario: A gate times out

- **WHEN** a tool gate exceeds its timeout
- **THEN** the call is decided by jcode's configured fallback policy, not by the plugin's silence

### Requirement: Gate fallback is explicit, not incidental

For plugin decisions that block or permit an action, jcode SHALL have a stated fallback for the
no-answer case, and the fallback MUST be configurable. The default MUST be documented in the generated
configuration and surfaced in diagnostics when it is applied.

#### Scenario: Default fallback is visible

- **WHEN** a gate does not answer
- **THEN** the applied fallback is reported in the session's diagnostics rather than being invisible

#### Scenario: Operator can require fail-closed

- **WHEN** the fallback is configured to deny
- **THEN** an unanswered gate blocks the call and says so

### Requirement: Plugins are listed and testable without starting a session

jcode SHALL provide a command to list configured plugins and to test one: start it, complete the
handshake, report its capabilities and version, and stop it. Testing MUST NOT require an interactive
session or a model call.

#### Scenario: Testing a plugin

- **WHEN** the operator tests a configured plugin
- **THEN** the handshake result, capabilities, and any failure are reported

#### Scenario: Listing

- **WHEN** the operator lists plugins
- **THEN** each configured plugin is shown with whether it is currently usable

### Requirement: Plugin output cannot corrupt the transcript or the cache

Text a plugin contributes SHALL enter the conversation only through the designated contribution channel,
MUST NOT be written into persisted history as if the user said it, and MUST NOT alter the cached prompt
prefix mid-session.

#### Scenario: Contributed context is not user history

- **WHEN** a plugin contributes context
- **THEN** it appears in the request to the model but not as a user message in the session's stored
  transcript

#### Scenario: The cached prefix is preserved

- **WHEN** a plugin contributes context on a later turn
- **THEN** the stable prompt prefix is unchanged and only the trailing contribution differs
