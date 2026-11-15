# Tasks

## 1. Entry points and identity

- [x] 1.1 Add the provider metadata entry (id `neuralwatt`, display name, base URL, key env, env file, setup
      URL, requires-key, default model) and verify it appears in `--provider`'s accepted values
- [x] 1.2 Verify `-p neuralwatt` parses and selects it, that the picker lists it, and that an unknown provider
      name still fails with the list of accepted values
- [x] 1.3 Implement the preset entry and confirm it is sufficient, recording the evidence in the design notes;
      if it is not (see design D1), stop and raise the variant fallback as a separate decision rather than
      absorbing 24 files of churn silently

## 2. Credential binding and login

- [x] 2.1 Add the dedicated env var and env file, and verify the key is read from them and that the generic
      compatible key is never consulted
- [x] 2.2 Add the login entry and verify a login writes the dedicated binding with owner-only permissions and
      reports success
- [x] 2.3 Verify logout clears only the Neuralwatt binding
- [x] 2.4 Verify an unconfigured Neuralwatt reports not-configured and leaves every other provider's status
      unchanged

## 3. Catalog

- [x] 3.1 Wire the live catalog fetch for Neuralwatt under its own cache namespace and verify it returns the
      models the endpoint reports (fixture-based test, no network in CI)
- [x] 3.2 Verify the cache is served on startup and refreshed in the background without emptying the list when
      the refresh fails
- [x] 3.3 Verify an api_base mismatch invalidates the cached catalog
- [x] 3.4 Verify the default model selection is deterministic for a catalog with no explicit default

## 4. Routing and attribution

- [x] 4.1 Route Neuralwatt model requests through the existing OpenAI-compatible runtime configured with the
      Neuralwatt endpoint, verified by a test asserting the request URL and auth header
- [x] 4.2 Verify a bare Neuralwatt model id resolves to Neuralwatt while another provider is active
- [x] 4.3 Verify a Neuralwatt id is never attributed to OpenRouter or to the generic compatible profile
- [x] 4.4 Verify crossing providers mid-session works in both directions (into and out of Neuralwatt)

## 5. Listing and selection surface

- [x] 5.1 Verify the model listing shows Neuralwatt models when it is active, labelled with Neuralwatt as the
      provider
- [x] 5.2 Verify the picker route entries carry Neuralwatt as the provider and that selecting one starts a
      request to the Neuralwatt endpoint
- [x] 5.3 Verify `provider-doctor neuralwatt` walks its checkpoints, or record why it is out of scope
- [x] 5.4 Verify the Anthropic-compatible alternative from design D2: a `[providers.neuralwatt]` entry with
      `type = "anthropic-compatible"`, `auth = "header"`, `auth_header = "x-api-key"` reaches `/v1/messages`
      and completes a request, and document that it is not `-p` addressable

## 6. Verification

- [x] 6.1 End-to-end: configure Neuralwatt by name, list its models, complete a streaming request with one
      tool call, and verify the tool result round-trips (the non-streaming half is the catalog request; jcode's
      chat transport is streaming-only, recorded in the design notes)
- [x] 6.2 Verify auto-detection: with only Neuralwatt configured it is selected; with several configured the
      default is unchanged from before the change
- [x] 6.3 Run `cargo test` for every touched crate and `cargo build` for the workspace; the preset added no
      `ActiveProvider` variant, so 0 match arms were forced
- [x] 6.4 Write `docs/PROVIDER_NEURALWATT.md` covering configuration, login, endpoint override, the
      one-time migration from the generic profile, and which of the two API surfaces is in use and why
- [x] 6.5 Verify no runtime crate was modified: `git diff --stat` touches only provider metadata, catalog
      wiring, config/docs, and tests

## Workflow follow-up

- Review the proposal, specs, design, and tasks before implementation.
- Confirm whether the preset-only shape (D1 alternative) is preferred after task 1.3's evidence.
- Archive the change once implementation and verification are complete.
