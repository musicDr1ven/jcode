# Proposal

## Why

A model id that belongs to a configured OpenAI-compatible profile can be bound to whichever provider
happens to be active, and the resulting request fails at that provider with its own "model does not
exist" error. The user sees a message that names no cause, the model never loads, and the same setup
works in another session where that profile was active first. The owner lookup that should prevent this
exists, but it only searches the session's route catalog, so it silently does nothing exactly when it is
needed.

## What Changes

- Resolve a model id against **every** configured OpenAI-compatible profile, not only the routes present
  in the current session.
- Never bind an unrecognised id to the active provider. An unresolvable id fails with an error that names
  the id and the candidates, instead of being handed to a provider that will reject it.
- Make a profile login mid-session **usable without restarting**: a login refreshes the catalog the running
  session resolves against.
- Make a wrong-provider rejection **actionable**: when a provider reports that a model does not exist and
  another configured provider serves that id, say so.
- Cover **named providers** (`[providers.<name>]`) as well as OpenAI-compatible profiles. Ownership today
  consults profiles only, so a model served by a named provider is exposed to exactly the same misattribution.

## Capabilities

### New Capabilities

- `jcode/model-provider-attribution`: how a bare model id is attributed to a provider — the resolution
  order, the cross-profile lookup, the refusal to guess, the effect of a mid-session login, and the
  diagnostics when a provider rejects a model another provider serves.

### Modified Capabilities

None. This is the second change in this repository; nothing has been archived yet.

## Impact

- **Touched code**: `crates/jcode-base/src/provider/mod.rs` (`openai_compatible_profile_owning_model`,
  the `set_model` fallback, the catalog memo key), `crates/jcode-base/src/provider/catalog_memo.rs`
  (invalidation), `crates/jcode-tui/src/tui/app/auth.rs` (login → catalog refresh),
  `crates/jcode-tui/src/tui/app/commands.rs` (error surfacing).
- **Behaviour change**: `/model <bare-id>` that resolves to an unconfigured or inactive provider now
  either switches to the owning profile or fails with a named error. Previously it silently retargeted
  the active provider.
- **No new dependency or crates.**
