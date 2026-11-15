# Design

## Context

Verified against the live endpoint and this workspace:

- Neuralwatt serves **two** API surfaces. `POST /v1/chat/completions` answers with the OpenAI error envelope
  (`{"error":{"message":...,"code":"invalid_api_key"}}`); `POST /v1/messages` answers with the Anthropic one
  (`{"type":"error","error":{"type":"authentication_error",...}}`). Both routes exist (GET returns 405 on
  both, i.e. wrong method rather than unknown path).
- Its live catalog reports 23 models with bare ids (`nw-flash`, `glm-5.3`, `qwen-3.8-27b`) and no vendor
  prefixes.
- The Anthropic runtime **is** repointable: `ANTHROPIC_BASE_URL` or a named profile selects the messages URL,
  and the auth header name is configurable (`x-api-key` by default). So the Anthropic transport is technically
  usable against Neuralwatt — this was checked, not assumed.
- jcode already has a **named provider table**, `[providers.<name>]`, with
  `type = "openai-compatible" | "anthropic-compatible" | "openrouter"`, plus `base_url`, `auth`,
  `auth_header`, `headers`, `api_key_env`, `env_file`, `default_model`, `model_catalog`, `models`, and
  `extra_body`. Either protocol is expressible **without any code**.
- **The actual gap**: `--provider` is a fixed `clap` value list. A probe with an isolated `JCODE_HOME` and a
  `[providers.neuralwatt]` entry returned
  `error: invalid value 'neuralwatt' for '--provider'`, then listed the 50-odd accepted names. Named providers
  are usable but not nameable.
- The generic `openai-compatible` profile path is what the attribution defect exploits, and
  `openai_compatible_profile_owning_model` never consults the named provider table.

## Goals / Non-Goals

**Goals:**

- `-p neuralwatt` works, with the credential, catalog, and attribution all its own.
- No new transport and no second runtime. The protocol decision reuses code that already works.
- Record the protocol choice with the evidence, so nobody re-litigates it from assumption.

**Non-Goals:**

- Porting or duplicating a runtime crate. `jcode-provider-openai-runtime` is 15,470 lines and
  `jcode-provider-anthropic-runtime` 7,935; duplicating either adds maintenance and no capability.
- A Neuralwatt-specific streaming implementation, tool protocol, or pricing model.
- Embedding or reranking endpoints.
- Making named providers globally addressable by `-p`; that is a broader change (see Open Questions).

## Decisions

### D1. A preset entry is the integration; a variant is the fallback

Neuralwatt enters through the preset mechanism that already backs the 45 existing named providers — the list
`--provider` validates against, which is why a preset is what makes `-p neuralwatt` work. That is a
provider-metadata entry plus a catalog namespace, and nothing else.

Rejected for now — **`ActiveProvider::Neuralwatt` variant**: it forces 25 exhaustive match arms across 24
files and buys nothing a preset does not, once attribution is fixed generically by the companion change. It
stays documented as the fallback if the preset proves insufficient — specifically, if Neuralwatt needs
provider-specific behaviour a preset cannot express (a non-compatible surface, provider-specific headers on
streaming, or pinning that the route layer refuses).

Rejected — **duplicating a runtime**: forking a 7.9k–15.5k line crate to obtain a separate identity is the
most expensive way to solve a naming problem.

### D2. OpenAI-compatible is the default surface, Anthropic-compatible is documented

The preset uses `/v1/chat/completions`: it is the path Neuralwatt already works through today (a
`provider-doctor` run passed every catalog checkpoint on `openai-compatible` pointed at this endpoint), and it
is what every other preset uses, so the preset machinery needs no special case.

The Messages API stays reachable **without code** through a `[providers.neuralwatt]` entry with
`type = "anthropic-compatible"`, which the design documents including the `auth_header = "x-api-key"`
requirement. That is the option for anyone who wants Anthropic-shaped semantics; it is not the default
because it forfeits `-p` addressability, which is the thing this change exists to fix.

If evidence later shows the Anthropic surface behaves better for this endpoint (for example if it exposes
prompt caching that the OpenAI surface does not — material given jcode's cache-first design), the default can
be flipped by changing the preset's `base_url`/type and documenting it, without touching a runtime.

### D3. Own credential binding

Dedicated variable and env file, following the paths other presets use. The generic `OPENAI_COMPAT_API_KEY` is
not consulted, so configuring Neuralwatt can never silently authenticate as a different endpoint. A user on
the generic profile logs in once; that is stated rather than silently supported, because a fallback here
would reintroduce the ambiguity this change removes.

### D4. Catalog under its own namespace

The live fetch and disk cache use the `neuralwatt` namespace through the existing
`load_disk_cache_entry_for_namespace` machinery, so Neuralwatt models cannot mix with the generic profile's
catalog. Staleness keeps serving the cache while a refresh runs, reusing the existing rule.

### D5. Auto-detection ordering is appended, never inserted

The availability flag is placed after the subscription providers, so adding Neuralwatt cannot change which
provider an existing user gets by default.

## Risks / Trade-offs

- **A preset cannot express everything** → the variant fallback is named and its trigger is stated, so the
  escalation path is a decision rather than a surprise.
- **Two documented protocols confuse operators** → one is the default and named in diagnostics; the other is
  documented with the reason to choose it.
- **The generic profile and the new provider both reach Neuralwatt** → deliberate during migration; the
  attribution change makes the first-class entry win for its ids, and the migration note says to log in once.
- **Upstream divergence** → a metadata entry is the smallest possible diff for a new provider; this is why the
  variant is deferred.
- **Bare-id attribution for this provider still depends on the companion change** → sequenced in the proposal;
  the preset does not by itself fix attribution.

## Migration Plan

Additive. An existing `openai-compatible.env` pointed at Neuralwatt keeps working; the new provider reads its
own file, so adopting it takes one login. Nothing is removed.

## Open Questions

- Whether named providers should be generically addressable as `-p <name>` rather than each needing a preset
  entry. That would make this change's preset unnecessary and is the more principled fix; it is a separate
  change because it touches provider-name validation everywhere.
- Whether the endpoint's Anthropic surface exposes prompt caching worth preferring. Deferrable: it only
  changes which surface the preset defaults to.
- Whether `provider-doctor` should cover presets generically rather than per-provider.
