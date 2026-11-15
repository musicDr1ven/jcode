# Spec Delta

## Purpose

Defines which provider a model id is attributed to, what happens when the id is ambiguous or unknown, and
what the operator is told — so that choosing a model selects the provider that actually serves it, and a
mismatch is diagnosable instead of appearing as a provider-side "model does not exist".

## ADDED Requirements

### Requirement: A model id is attributed to the provider that serves it

When a model id is served by a configured provider other than the currently active one, resolving that id
SHALL select the serving provider. The active provider MUST NOT be assumed to serve an id it does not list.

#### Scenario: Bare id owned by an inactive profile

- **WHEN** an OpenAI-compatible profile is configured and its catalog lists `some-bare-id`, and a
  different provider is active
- **THEN** selecting `some-bare-id` switches to that profile and routes the request there

#### Scenario: Active provider is not consulted for ownership

- **WHEN** the active provider does not list the id
- **THEN** that fact does not make the id unowned

### Requirement: An unowned id is never guessed across providers

When the active provider is an aggregator or a compatible endpoint, and no configured provider's catalog
serves a model id, resolution SHALL fail with an error naming the id rather than binding it to the active
provider. A built-in provider MAY continue to accept ids in its own opaque space, because it owns that
space and no catalog of another provider can claim it.

#### Scenario: Unknown id under an aggregator is refused

- **WHEN** an id matches no configured provider's catalog and an aggregator or compatible endpoint is active
- **THEN** the switch fails before any request is sent, and the error names the id

#### Scenario: No cross-provider rejection is produced for an unowned id

- **WHEN** an unknown id is submitted with an aggregator active
- **THEN** no model request is issued to any provider

#### Scenario: A built-in provider keeps its own id space

- **WHEN** a built-in provider is active and the id matches no heuristic but belongs to that provider
- **THEN** the id is accepted for that provider rather than refused

### Requirement: Only the serving provider's catalog decides ownership

Ownership SHALL be determined by the cache and live catalog of each configured provider, preferring the
active one when several serve the same id. Static name heuristics MUST NOT override a catalog that lists
the id, and MUST NOT claim an id that no catalog lists.

#### Scenario: Ambiguous id prefers the active provider

- **WHEN** two configured providers both list the same id and one is active
- **THEN** the active one is chosen, so re-selecting the current model never hops endpoints

#### Scenario: Heuristic does not claim profile ids

- **WHEN** a bare id has no vendor prefix and a profile's catalog lists it
- **THEN** the profile owns it rather than a name-pattern heuristic

### Requirement: Ownership covers every configured provider kind

Ownership SHALL consider OpenAI-compatible profiles, named providers configured in the provider table, and
first-class providers. A model served by a named provider MUST resolve to that provider rather than to the
active one.

#### Scenario: Named provider owns a bare id

- **WHEN** a `[providers.<name>]` entry's catalog lists an id and a different provider is active
- **THEN** selecting that id selects the named provider

#### Scenario: Named provider is not treated as unowned

- **WHEN** the only provider serving an id is a named provider
- **THEN** the id is not reported as unowned

### Requirement: A login takes effect in a running session

Completing a login for a provider SHALL make that provider's models resolvable in already-running sessions
without a restart.

#### Scenario: Profile logged in mid-session

- **WHEN** an operator logs into an OpenAI-compatible profile in a session started on another provider
- **THEN** that profile's models become selectable in the same session

#### Scenario: Catalog refresh follows the login

- **WHEN** the login completes
- **THEN** the session's route catalog includes the new provider's models

### Requirement: A provider rejection names the likely cause

When a provider reports that a model does not exist, and another configured provider serves that id, the
error SHALL name the owning provider and how to select it. It MUST NOT present the provider's raw message
as the whole explanation.

#### Scenario: Wrong-provider 404 is explained

- **WHEN** a request is rejected because the id belongs to another configured provider
- **THEN** the message names that provider and the action to switch to it

#### Scenario: Genuinely unknown model keeps the raw cause

- **WHEN** no configured provider serves the id
- **THEN** the provider's message is surfaced, alongside the statement that no provider serves it

### Requirement: Attribution is covered by a regression test

The resolution path SHALL be covered by tests that fail if a bare id is ever attributed to a provider that
does not list it.

#### Scenario: Regression test exists

- **WHEN** the resolution logic is changed
- **THEN** a test asserts that a bare profile id never resolves to a non-serving active provider
