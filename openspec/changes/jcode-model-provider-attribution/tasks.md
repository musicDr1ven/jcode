# Tasks

## 1. Reproduce and pin the defect

- [x] 1.1 Reproduce with two configured providers: start a session on provider A, then select a bare id owned
      only by provider B's catalog, and record the exact error and log lines in
      `docs/model-attribution-repro.md`
- [x] 1.2 Add a failing regression test asserting a bare id owned only by an inactive profile does not resolve
      to the active provider, and confirm it fails before the fix
- [x] 1.3 Verify the same scenario is unaffected for ids that carry a vendor prefix or `/`, so the fix cannot
      regress the working paths

## 2. Cross-profile ownership

- [x] 2.1 Extend `openai_compatible_profile_owning_model` with a catalog pass over all configured profiles,
      preferring the active profile then declaration order, verified by a test with two profiles where only
      the inactive one serves the id
- [x] 2.2 Verify the session-route fast path still wins when it owns the id, so no extra disk read occurs on a
      resolved id
- [x] 2.3 Verify an ambiguous id served by two profiles prefers the active one, and that re-selecting the
      current model never changes endpoint

## 2b. Named providers

- [x] 2b.1 Confirm a `[providers.<name>]` entry serving a bare id is currently unattributed to that provider,
      by reproducing it with an isolated `JCODE_HOME` and recording the result
- [x] 2b.2 Extend the ownership pass to named providers through the same interface as profiles, verified by a
      test where the only provider serving an id is a named provider
- [x] 2b.3 Verify a named provider and a profile that serve the same id resolve deterministically, and record
      the precedence in `docs/MODEL_ATTRIBUTION.md`

## 3. Refuse instead of guessing

- [x] 3.1 Replace the active-provider fallback in `set_model` with an error naming the id, scoped to the
      aggregator/compatible slot, verified by a test asserting the provider is unchanged after the refusal
- [x] 3.1b Add a test asserting a built-in provider still accepts an id in its own opaque space, so the
      carve-out cannot silently widen into "resolve everything through catalogs"
- [x] 3.2 Verify every known-prefix path (vendor families, `org/model` ids, `@pin` routes) still resolves
      without reaching the new error
- [x] 3.3 Verify the error message text is actionable in one line and includes at least one configured
      provider name or states that none exist

## 4. Mid-session login takes effect

- [x] 4.1 Trigger an immediate catalog sweep when credentials change (the auth generation bump already
      exists), verified by a test that a bare id becomes resolvable after a simulated login without waiting
      for the scheduler interval
- [x] 4.2 Verify the running session's picker reflects the new provider's models without restart
- [x] 4.3 Verify a logout also invalidates so a removed provider's ids stop resolving

## 5. Actionable rejection messages

- [x] 5.1 On a provider model-not-found rejection, check whether another configured provider serves the id and
      enrich the message with that provider's name and the switch action, verified for both the served-elsewhere
      case and the served-nowhere case
- [x] 5.2 Verify the provider's raw message is preserved in the enriched error
- [x] 5.3 Verify the enrichment does not run on unrelated failures (auth, rate limit, payload size)

## 6. Verification

- [x] 6.1 End-to-end: with provider A active and provider B logged in later, switch to a B-only model in the
      same session and complete a request successfully
- [x] 6.2 Verify the regression test from 1.2 now passes
- [x] 6.3 Run `cargo test` for every touched crate (`jcode-base`, `jcode-tui`) and confirm no existing
      provider-attribution test changed
- [x] 6.4 Record the fix and the invariant it protects in `docs/MODEL_ATTRIBUTION.md` so the next reader does
      not reintroduce the fallback

## Workflow follow-up

- Review the proposal, specs, design, and tasks before implementation.
- Decide whether this is proposed upstream, and record the decision where the next rebase will find it.
- Archive the change once implementation and verification are complete.
