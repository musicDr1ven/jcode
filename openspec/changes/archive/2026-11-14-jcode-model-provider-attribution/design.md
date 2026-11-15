# Design

## Context

See `proposal.md` for motivation; `specs/` for the contract. Verified in this workspace:

- `provider_for_model_with_hint` (`crates/jcode-base/src/provider/models.rs:1299`) attributes by name
  pattern only: `@` → openrouter, known Claude/OpenAI families, bedrock ids, **any id containing `/` →
  openrouter**, then known prefixes, then `None`. It has no branch for OpenAI-compatible profiles.
- `set_model` (`crates/jcode-base/src/provider/mod.rs:2204`) tries `provider_for_model`, then
  `openai_compatible_profile_owning_model`, then **falls back to the active provider**.
- `openai_compatible_profile_owning_model` (`mod.rs:865`) searches only
  `fresh_routes_memo_entry().routes`, i.e. this session's catalog, and requires `route.available`.
- That memo has a 60 s TTL and is invalidated by `auth_pricing_generation()` and `catalog_generation()`.
- Observed behaviour: `-p openai-compatible model list` shows 14 bare profile ids; `-p openrouter model
  list` shows none of them, so under OpenRouter the id is visible nowhere and the owner lookup cannot
  succeed from session routes alone.
- A user-facing "model does not exist" is an upstream rejection recognised by
  `is_fatal_model_endpoint_error` (`crates/jcode-tui/src/tui/app/commands.rs:170`), i.e. the request
  reached a provider that does not serve the id.

## Goals / Non-Goals

**Goals:**

- Ownership decided by catalogs, never by "the one that happens to be active".
- A wrong-provider failure becomes a one-line diagnosis rather than a dead end.
- Logins take effect immediately in a running session.

**Non-Goals:**

- Changing which models the picker *displays* while scoped to a provider. Discovery is a separate concern
  from resolution; this change makes `/model <id>` work, not the picker's list.
- Adding a provider (that is the companion change).
- Reworking the name heuristics for vendors that legitimately use `org/model` ids.

## Decisions

### D1. Ownership is a catalog question, answered across all profiles

`openai_compatible_profile_owning_model` gains a second pass: if no session route owns the id, consult each
configured profile's own catalog via `cached_live_models_for_openai_compatible_profile`, preferring the
active profile and then declaration order. Session routes stay the fast path.

Rationale: the catalog is per profile and already cached; the session route list is a *view* and was never
meant to be the authority. Alternatives rejected: widening every session's route build to include all
profiles (cost on a 2000+ route catalog for every render), and trusting `provider_for_model` name patterns
(cannot recognise arbitrary bare ids by construction).

### D2. The active provider is no longer a fallback for unresolved ids — except where it owns the space

Finding from implementation: refusing unconditionally broke two existing bedrock placeholder tests, because a
built-in provider legitimately issues ids that no name heuristic recognises
(`us.anthropic.claude-opus-4-6-v1`). Those ids are not "unknown"; they belong to the active provider's own
namespace.

So the refusal is scoped to the case that actually produced the defect: the native OpenRouter aggregator
slot, where a bare id belonging to a compatible profile was accepted and turned into a cross-provider
rejection. Direct OpenAI-compatible endpoints keep the fallback too: a custom or local server may never
publish a `/models` catalog, so refusing unlisted ids would make its local models unusable. Built-in
providers keep their opaque space for the same reason. In all three cases ownership runs first, so an id
another configured provider serves is claimed before any fallback, and the spec states these bounds rather
than pretending the rule is absolute.

The alternative — resolving built-in providers' opaque ids through explicit ownership too — is real work in
`bedrock`/`copilot`/`gemini` placeholder paths for no user-visible gain.

The final `else` in `set_model` changes from "set on active" to an error naming the id and the considered
providers. This is the crux of the defect: a guess is worse than a refusal, because the guess sends a
request that fails far from the cause.

Alternative considered: keep the fallback for backwards compatibility. Rejected — the fallback is what
produces the misleading provider error, and known-prefix ids still resolve without it.

### D2b. Named providers join the same ownership pass

Ownership today is a profile question. The provider table (`[providers.<name>]`, supporting
`openai-compatible` and `anthropic-compatible` types) is a second, equally valid source of models, and
`openai_compatible_profile_owning_model` never looks at it. The catalog pass in D1 therefore iterates
profiles *and* named providers through one ownership interface, so the invariant is stated once.

Note a separate fact that constrains the companion change: `--provider` is a fixed `clap` value list, so a
named provider is configurable and usable but **not addressable by `-p <name>`**. That gap belongs to the
Neuralwatt change, not here.

### D3. A login bumps the catalog generation

`CatalogGeneration` is already the invalidation channel for the routes memo. Completing a login for any
provider bumps it, so the running session's next resolution rebuilds with the new provider's routes. This
is smaller than an event-based push and reuses machinery that exists.

### D4. The rejection message is enriched at the TUI layer

When a provider rejects a model, the error path checks whether any configured provider serves the id and,
if so, names it. The provider's own text stays in the message — it is the evidence — with the likely cause
added around it.

Chosen over suppressing the raw message: the raw text is what a maintainer needs when the enrichment is
wrong.

### D5. Regression test targets the invariant, not the code path

The test asserts the *invariant* — a bare id owned only by an inactive profile never resolves to the active
provider — by driving the public resolution entry point with a fixture catalog, so refactoring the
implementation does not silently drop the guarantee.

## Risks / Trade-offs

- **Extra catalog reads on a miss** → only on the miss path (a resolved id never pays), and each profile's
  catalog is already cached on disk.
- **Ambiguity across profiles** → deterministic preference (active first, then declaration order), which the
  spec states rather than leaves to chance.
- **Refusing unknown ids breaks a workflow that relied on the fallback** → those requests were failing at the
  provider anyway; the new error is strictly more informative and arrives before the request.
- **Catalog generation churn** → a bump on login invalidates route memos in all sessions; acceptable because
  logins are rare and the rebuild is memoized.

## Migration Plan

No persisted format changes. Behaviour changes only for ids that previously mis-resolved; a session that
worked keeps working because the owning provider is still selected, just via the catalog instead of the
active-provider fallback.

## Open Questions

- Whether the picker should gain an "all providers" mode so discovery matches resolution. Deferrable: it is
  additive UI over the same ownership data.
- Whether `model list` should accept an `--all` flag for the same reason. Deferrable.
