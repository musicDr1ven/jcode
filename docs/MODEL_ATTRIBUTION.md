# Model attribution

How a model id becomes a provider switch, and the invariant that keeps a
provider from being handed an id it does not serve.

## The invariant

**A bare model id is owned by a provider whose catalog lists it, and is never
guessed onto the active provider across provider kinds.**

The active provider is never evidence that it serves an id. Ownership comes from
each configured provider's own catalog (live cache, static list, or declared
default), never from "whichever provider happens to be active".

## Resolution order (`MultiProvider::set_model`)

1. **Explicit routing directives win.** `@pin` routes, vendor-family prefixes,
   `org/model` ids, `<profile>:<model>`, `<named-provider>:<model>`, and
   `jcode:<model>` are handled before any ownership lookup. A prefixed id is an
   instruction, not a question.
2. **Session routes are the fast path**
   (`openai_compatible_profile_owning_model`). The route catalog carries the
   active runtime's own catalog, which can be fresher than the on-disk snapshot.
   The active profile wins here too.
3. **Catalog pass over every configured provider** (`catalog_owning_model`):
   - the active provider first, whether it is an OpenAI-compatible profile or a
     `[providers.<name>]` entry;
   - then configured OpenAI-compatible profiles in declaration order;
   - then configured named providers in name order.

   Profiles are checked against the same route data the picker shows (live disk
   cache, else static models, else the declared default), so ownership and
   discovery cannot disagree about what a provider serves.
4. **No owner:**
   - with the native OpenRouter aggregator active, resolution **fails** before any
     request is sent, naming the id and the configured providers;
   - with a direct OpenAI-compatible endpoint active (including a configured
     profile holding the slot), the id stays on that endpoint, because custom
     local servers issue opaque ids that no catalog lists; the ownership guard
     runs first, so an id another provider serves is still claimed by it;
   - with a built-in provider active, the id stays in that provider's own opaque
     space (`bedrock`, `copilot`, and `gemini` issue ids no name heuristic or
     catalog recognises — e.g. `us.anthropic.claude-opus-4-6-v1`).

## Precedence when several providers serve the same id

1. The **active provider wins**. Re-selecting the current model therefore never
   hops endpoints, whichever provider kind is active.
2. Otherwise an OpenAI-compatible **profile** is preferred over a **named
   provider**.
3. Otherwise declaration order (profiles) and name order (named providers)
   decide.

The result is deterministic in every case: the same configuration and the same
active provider always resolve to the same owner.

## Mid-session login and logout

Credentials decide which providers are configured, so `AuthStatus::invalidate_cache()`
is the single choke point for credential changes:

- it bumps the auth pricing generation, which already invalidates every
  session's route memo;
- it now also calls `catalog_scheduler::sweep_stale_profile_catalogs_now()`.

The sweep is what makes a *newly configured* provider usable immediately. Route
building is deliberately pure (it never fetches), so without an immediate sweep a
profile logged in mid-session would have no live catalog until the next
scheduled sweep, up to a minute later. The sweep only schedules refreshes for
stale or missing caches and is a no-op when everything is fresh.

Logout is covered by the same path: ownership requires the provider to be
configured, so cache files left behind by a logged-out provider stop conferring
ownership and its ids are refused.

## Rejection diagnostics

When a provider reports that a model does not exist, the TUI keeps the
provider's own compacted text (it is the evidence) and appends one line:

- served elsewhere: `` `<id>` is served by <Provider>; switch with `/model <prefix>:<id>`. ``
- served nowhere: `` No configured provider lists `<id>`; check the id. ``

The data comes from `provider::configured_model_owner`, which uses the same
profile/named-provider catalog data as resolution. The enrichment is gated on
`is_fatal_model_endpoint_error`, so auth, rate-limit, and payload-size failures
are displayed unchanged.

## Why the active-provider fallback must not return

The previous `set_model` `else` bound every unrecognised id to the active
provider. For a bare id belonging to another configured provider that produced a
request to the wrong endpoint and a provider-side "model does not exist",
several layers away from the actual cause. A refusal is strictly better: it
arrives before the request, names the id, and lists the configured providers.

## Tests

`crates/jcode-base/src/provider/tests/model_resolution.rs`:

| Test | Protects |
| --- | --- |
| `bare_profile_id_owned_only_by_an_inactive_profile_never_resolves_to_the_active_endpoint` | A B-only id resolves to B while A is the active direct endpoint (the original defect). Fails if the ownership guard before the custom-endpoint branch is removed. |
| `test_unowned_bare_model_is_refused_instead_of_binding_to_active_provider` | Refusal on the aggregator slot; provider and model unchanged. |
| `unowned_model_refusal_names_a_configured_provider` | The refusal names a configured provider and stays one line. |
| `test_routed_model_ids_still_resolve_after_the_refusal_was_added` | `org/model` and other routed ids never hit the refusal. |
| `test_builtin_provider_is_not_subjected_to_the_catalog_refusal` | The built-in carve-out. |
| `ambiguous_profile_id_prefers_the_active_profile` | Active-first ambiguity; re-selection never hops. |
| `named_provider_owns_a_bare_id_while_another_provider_is_active` | Named providers participate in ownership. Fails if the catalog pass is removed. |
| `profile_and_named_provider_precedence_is_deterministic` | Profile-over-named default, active-over-everything when active. |
| `login_makes_a_newly_configured_profiles_models_resolvable_without_restart` | Credential change triggers the immediate sweep; the picker then lists the new models. |
| `logout_stops_resolving_a_removed_providers_ids` | A logged-out provider's cached catalog stops conferring ownership. |
| `configured_model_owner_names_the_serving_provider_and_switch_prefix` | The rejection hint names a provider and an addressable switch prefix. |
| `a_provider_configured_mid_session_serves_a_bare_id_end_to_end` | Full path: switch to a mid-session provider's bare id and complete the request there. |

`crates/jcode-tui/src/tui/app/commands_tests.rs` covers the rejection
enrichment: served elsewhere, served nowhere, raw text preserved, the active
provider not blamed, and no enrichment for auth/rate-limit/payload failures.
