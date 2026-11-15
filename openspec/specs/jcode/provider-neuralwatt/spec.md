# jcode/provider-neuralwatt Specification

## Purpose

Defines Neuralwatt as a provider of its own rather than a preset profile: how it is selected, where its
credential lives, how its catalog is obtained, which provider its models belong to, and what happens when
it is unconfigured or unreachable.

## Requirements

### Requirement: Neuralwatt is selectable by name

The provider SHALL be addressable as `neuralwatt` wherever a provider can be named — start-up selection and
in-session switching — and SHALL appear in the provider list with a display name an operator can recognise.

#### Scenario: Selected at start-up

- **WHEN** a session starts with the provider set to `neuralwatt` and a credential is present
- **THEN** the session runs on Neuralwatt without falling back to another provider

#### Scenario: Selected in session

- **WHEN** an operator switches to Neuralwatt mid-session
- **THEN** subsequent requests go to the Neuralwatt endpoint

### Requirement: Neuralwatt has its own credential binding

Its API key SHALL be read from its own environment variable and its own configuration file, and MUST NOT
require the generic OpenAI-compatible variable. A login entry SHALL write that binding.

#### Scenario: Own credential is used

- **WHEN** the Neuralwatt key is configured in its own binding
- **THEN** requests authenticate with it and the generic compatible key is not consulted

#### Scenario: Login writes the binding

- **WHEN** the operator logs in to Neuralwatt
- **THEN** the credential is stored in Neuralwatt's file and reported as configured

#### Scenario: Absence does not affect other providers

- **WHEN** no Neuralwatt credential is configured
- **THEN** Neuralwatt reports not-configured and every other provider is unaffected

### Requirement: Its models are attributed to Neuralwatt alone

Every model id in Neuralwatt's catalog SHALL resolve to Neuralwatt, including bare ids with no vendor
prefix and including while a different provider is active.

#### Scenario: Bare id while another provider is active

- **WHEN** a Neuralwatt-only model id is selected while another provider is active
- **THEN** the request is routed to Neuralwatt

#### Scenario: Not attributed to a name-pattern heuristic

- **WHEN** a Neuralwatt model id resembles another vendor's naming
- **THEN** Neuralwatt's catalog still decides ownership

### Requirement: The catalog comes from the provider and is cached

The model list SHALL be fetched live from the Neuralwatt endpoint and cached under its own namespace, and
MUST survive a restart by serving the cache while a refresh runs. A stale cache MUST still serve its models
rather than emptying the list.

#### Scenario: Live fetch populates the catalog

- **WHEN** Neuralwatt is configured and reachable
- **THEN** its catalog contains the models the endpoint reports

#### Scenario: Stale cache still serves

- **WHEN** the cached catalog is older than the refresh interval and the endpoint is unreachable
- **THEN** the cached models remain selectable and the staleness is reported

#### Scenario: Failure does not empty the catalog

- **WHEN** a refresh fails
- **THEN** the previous catalog stays intact

### Requirement: The endpoint is configurable with a working default

The base URL SHALL default to the Neuralwatt API and SHALL be overridable per installation, with the
override taking effect without code changes.

#### Scenario: Default endpoint

- **WHEN** no override is configured
- **THEN** requests go to the default Neuralwatt base URL

#### Scenario: Override applies

- **WHEN** an override is configured
- **THEN** both the catalog fetch and model requests use it

### Requirement: Its models are listed under Neuralwatt

Neuralwatt's models SHALL appear in the model listing when Neuralwatt is the active provider, and SHALL be
listed under Neuralwatt rather than under a generic compatible provider.

#### Scenario: Listing while Neuralwatt is active

- **WHEN** Neuralwatt is active
- **THEN** its models are listed without being mixed into another provider's list

#### Scenario: Labelled with the owning provider

- **WHEN** a model's routes are shown
- **THEN** Neuralwatt models name Neuralwatt as the provider
