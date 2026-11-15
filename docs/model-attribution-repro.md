# Model attribution: observed defect and invariant

## Symptom

A model id belonging to a configured OpenAI-compatible profile is rejected by whichever provider is
active, with that provider's own "model does not exist" error. The model never loads and the message
names no cause. Reproduced on macOS after logging into a second provider from a session started on the
first; the same setup worked when the serving provider was active first.

## Two causes, both in bare-id resolution

1. **An early return bound every unprefixed id to the active compatible endpoint**, before ownership was
   consulted (`set_model`, the "custom endpoint owns opaque ids" branch). This is the branch that actually
   produced the symptom.
2. **Ownership was answered from the session's route list**, which is scoped by the active provider, so a
   profile the session was not using was invisible to
   `openai_compatible_profile_owning_model`. `[providers.<name>]` entries were never consulted at all.

Evidence: `jcode -p openai-compatible model list` lists the profile's bare ids; `jcode -p openrouter
model list` lists none of them. Under the aggregator the id is visible nowhere, so the route-scoped owner
lookup cannot succeed.

## The invariant this change protects

**A bare model id is owned by a provider whose catalog lists it, and is never guessed onto the active
provider across provider kinds.** Ownership order: session routes (fast path) → each profile's own cached
catalog, active profile first then declaration order → configured named providers.

One carve-out, deliberate: a direct OpenAI-compatible endpoint (including a configured profile holding the
slot) and a built-in provider keep their own opaque id space. Bedrock issues ids such as
`us.anthropic.claude-opus-4-6-v1` that no name heuristic recognises, and a local compatible server may never
publish a catalog at all; refusing those broke existing placeholder tests and local-endpoint workflows. The
refusal therefore applies where guessing across providers was the harm — the native OpenRouter aggregator
slot — and the ownership guard runs before the opaque-id branch so another provider's id is still claimed.

## Tests

`crates/jcode-base/src/provider/tests/model_resolution.rs`:

- `bare_profile_id_owned_only_by_an_inactive_profile_never_resolves_to_the_active_endpoint` — the
  regression test (task 1.2), driven through `set_model`: a Meta-only bare id must move the session off
  an active Celeris endpoint. Verified red by removing the ownership guard in `set_model` and green with
  it restored.
- `test_unowned_bare_model_is_refused_instead_of_binding_to_active_provider` — no provider switch, no model
  change, error names the id.
- `test_routed_model_ids_still_resolve_after_the_refusal_was_added` — `org/model` ids still route.
- `test_builtin_provider_is_not_subjected_to_the_catalog_refusal` — the carve-out.
- `ambiguous_profile_id_prefers_the_active_profile` — active profile wins an id served by two profiles.
- `named_provider_owns_a_bare_id_while_another_provider_is_active` — task 2b.1/2b.2; verified red by making
  the ownership pass route-only (the pre-change behaviour), green with the catalog pass.
- `profile_and_named_provider_precedence_is_deterministic` — task 2b.3.
- `unowned_model_refusal_names_a_configured_provider` — task 3.3.
- `login_makes_a_newly_configured_profiles_models_resolvable_without_restart` and
  `logout_stops_resolving_a_removed_providers_ids` — tasks 4.1-4.3.
- `configured_model_owner_names_the_serving_provider_and_switch_prefix` — task 5.1 data.
- `a_provider_configured_mid_session_serves_a_bare_id_end_to_end` — task 6.1.

Note on regression proof: every test asserts through public entry points (`set_model`, `model_routes`,
`complete`), so reverting the fix fails an assertion rather than merely breaking compilation.

## Named providers were unattributed (task 2b.1)

Reproduced with an isolated `JCODE_HOME`: a `[providers.gateway]` entry's model appears in the session
routes with `api_method = "openai-compatible:gateway"`. The owner lookup mapped that api method back
through `openai_compatible_profile_by_id`, which only knows catalog profiles, so a named provider's id
came back unowned and `set_model` fell through to the active provider. The catalog pass now resolves
named providers too; making the pass route-only reproduces the failure.

## Verification runs

- `cargo test --release -p jcode-base --lib`: 1945 passed / 35 failed with this change; 1934 passed / 34 failed on
  clean `HEAD`. The 12 new tests account for the 11 extra passes plus one timing flake,
  `mcp::remote::remote_tests::remote_handshake_deadline_is_independent_of_long_tool_timeout`, which passes in
  isolation. Every other failure is in the pre-existing list below.
- `cargo test --release -p jcode-base --lib provider::`: 7 failures, all in the pre-existing list below.
- `jcode-tui`: the full lib suite does not complete in this environment. It stalls in
  `tui::ui::tests::swarm_buffer` / `pinned_todo_regressions` after ~2,870 passing tests, and the same stall
  reproduces on clean `HEAD` with no local changes. Verification was therefore scoped to the touched modules:
  `tui::app::commands` reports 85 passed on clean `HEAD` and 90 with this change (the 5 new enrichment tests),
  0 failures; `tui::app::remote` reports 110 passed, 0 failures; `tui::ui::tests::swarm_buffer` run serially
  reports the same 2 pre-existing layout failures on both trees.

## Known pre-existing failures (not caused by this change)

Verified by stashing the change and re-running on a clean tree:

- `provider::bedrock_placeholder_default_tests::bedrock_placeholder_promotes_to_newest_usable_claude`
- `provider::bedrock_placeholder_default_tests::new_bedrock_provider_starts_on_newest_cached_flagship`
- `provider::tests::test_subscription_model_guard_allows_only_curated_models_when_enabled`
  (asserts `claude-sonnet-4-6` is *not* curated; the curated list in that test has drifted)
- `provider::tests::test_openai_model_catalog_is_scoped_per_account`
- `provider::tests::test_openai_provider_unavailability_is_scoped_per_account`
- `provider::tests::test_openai_reset_clears_only_pinned_account_cooldown`
- `provider::tests::test_openai_reset_default_scope_does_not_follow_active_account`

The last four fail on clean `HEAD` as well (confirmed by reverting only the two touched files and running
them in isolation), so `--lib provider::` is expected to report 7 pre-existing failures.

## Mid-session login: premise corrected (task 4.1)

The plan assumed a login needed a `catalog_generation` bump to take effect. It does not:
`AuthStatus::invalidate_cache()` already calls `provider::pricing::invalidate_auth_pricing_memos()`,
which bumps `AUTH_PRICING_GENERATION`, and the routes memo snapshots both that and
`catalog_generation`. So a login already invalidates every provider's memoized catalog.

What a login did *not* do is populate the new profile's on-disk catalog: that is the sweeper's job, on a
60 s interval. So the residual gap was that a mid-session login's dynamically-fetched models stayed
unowned for up to a minute. Task 4.1 was re-scoped accordingly (not dropped): `invalidate_cache()` now
also calls `catalog_scheduler::sweep_stale_profile_catalogs_now()`, so the newly configured profile's
catalog refresh is scheduled with the credential change instead of waiting for the interval. The refresh
itself is asynchronous, which is why the test waits for the cache rather than expecting it synchronously.

