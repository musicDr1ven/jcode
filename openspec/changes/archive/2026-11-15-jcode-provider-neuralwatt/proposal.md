# Proposal

## Why

Neuralwatt exposes **two** API surfaces — an OpenAI-compatible `/v1/chat/completions` and an
Anthropic-compatible `/v1/messages` (both verified by their distinct error envelopes) — but jcode can reach
it only through the generic `openai-compatible` profile: one shared slot, one shared env file, and one shared
attribution path. That is why its bare model ids share a fate with any other profile — a session on another
provider can bind one of them to the wrong endpoint and fail with a provider-side "model does not exist".

The specific gap is **addressability**. `--provider` is a fixed value list, and named providers configured in
`[providers.<name>]` — which can already target either protocol — are not in it. So Neuralwatt cannot be
named on the command line, has no login entry of its own, and cannot be selected as a distinct provider.

Making it first-class gives it an identity of its own: `-p neuralwatt`, its own credential binding, its own
catalog, and model ids that cannot be attributed to another provider.

## What Changes

- Add **Neuralwatt** to the named provider list so it is addressable as `-p neuralwatt` and selectable like
  any other named provider, starting from the preset mechanism that already backs every other entry in that
  list.
- Keep the protocol a **documented choice**: the preset uses the OpenAI-compatible surface that is proven
  end-to-end today, and the Anthropic-compatible surface stays reachable through a `[providers.neuralwatt]`
  entry for anyone who wants the Messages API.
- Give it its **own credential binding** — a dedicated env var and env file, plus a login entry — instead of
  piggybacking on `openai-compatible.env`.
- Give it its **own model catalog**: live fetch from the Neuralwatt endpoint, cached under its own namespace,
  with its own default model selection.
- Ensure its model ids are **attributed to Neuralwatt only**, including while another provider is active.

## Capabilities

### New Capabilities

- `jcode/provider-neuralwatt`: what it means for Neuralwatt to be a first-class provider — addressability,
  credential binding and login, catalog and default model, model attribution, endpoint configuration, and
  behaviour when it is unconfigured or unreachable.

### Modified Capabilities

None. Nothing in this repository has been archived yet.

## Impact

- **New code**: a provider-metadata entry plus a catalog namespace — the same shape as the 45 existing
  presets. **No new transport and no duplicated runtime**: the OpenAI-compatible path already carries
  Neuralwatt end to end.
- **Deferred, not required**: an `ActiveProvider::Neuralwatt` variant. It would force 25 exhaustive match arms
  across 24 files for no capability gain once attribution is fixed generically, so it is held as the fallback
  if the preset proves insufficient (see design D1).
- **Migration**: an existing `openai-compatible.env` pointed at Neuralwatt keeps working; the new provider
  reads its own file, so a user must log in once to adopt it.
- **Sequencing**: depends on `jcode-model-provider-attribution` for the general refusal-not-guess behaviour;
  this change makes one class of that defect structurally impossible for this provider.
