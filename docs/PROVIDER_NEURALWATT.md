# Neuralwatt provider

Neuralwatt Cloud is a hosted inference service with an OpenAI-compatible
`/v1/chat/completions` surface and an Anthropic-compatible `/v1/messages`
surface. jcode ships it as a first-class preset: `-p neuralwatt`, its own
credential binding, its own catalog, and model ids that are attributed to
Neuralwatt alone.

## Configuration

| Item | Value |
| --- | --- |
| Provider id | `neuralwatt` |
| Display name | `Neuralwatt` |
| Base URL (default) | `https://api.neuralwatt.com/v1` |
| API key variable | `NEURALWATT_API_KEY` |
| Env file | `neuralwatt.env` |
| Setup | <https://portal.neuralwatt.com> (Dashboard → API Keys) |

Log in once to create the binding:

```bash
jcode login --provider neuralwatt
```

Paste the key when prompted (keys start with `sk-`). The key is written to
`neuralwatt.env` in jcode's config directory with owner-only permissions, and
never into the process environment (issue #1386). The interactive equivalent is
`/login` → Neuralwatt.

The generic `OPENAI_COMPAT_API_KEY` binding is deliberately **not** consulted,
so a Neuralwatt login can never silently authenticate as a different endpoint,
and logging out of Neuralwatt clears only its own file.

Start a session on it, or switch mid-session:

```bash
jcode -p neuralwatt
```

```text
/model neuralwatt:nw-flash
```

## Endpoint override

The base URL can be pointed at a proxy or a regional endpoint without a code
change, either in the environment or persisted in `neuralwatt.env`:

```bash
export JCODE_NEURALWATT_API_BASE="https://neuralwatt-proxy.example.com/v1"
```

The override applies to **both** the catalog fetch and model requests. Changing
it invalidates the previously cached catalog (the cache records its source base
URL), so models from the old endpoint are never mistaken for the new one's.

## Model catalog

The model list is fetched live from `GET /v1/models` and cached under
Neuralwatt's own namespace (`neuralwatt_models.json`), so it never mixes with
the generic `openai-compatible` profile's catalog.

- A stale cache still serves its models while a background refresh runs; a
  failed refresh leaves the previous catalog intact.
- Neuralwatt reports `created: 0` for every model, so the automatic default is
  chosen deterministically: strongest quality tier first, then catalog order.
  Pick a specific model with `/model` or `--model`.
- List the current catalog with:

  ```bash
  jcode -p neuralwatt model list
  ```

## Model attribution

Every id in Neuralwatt's catalog resolves to Neuralwatt, including bare ids
with no vendor prefix (`nw-flash`, `glm-5.3`, `qwen-3.8-27b`) and while a
different provider is active. The generic `openai-compatible` profile and the
OpenRouter aggregator never claim one of these ids. Crossing into and out of
Neuralwatt mid-session is a normal `/model` switch.

## The two API surfaces

**OpenAI-compatible (default).** The preset uses `/v1/chat/completions`. This is
the surface every other preset uses, it is what `provider-doctor` already
exercised against this endpoint, and no runtime special-casing is needed.

**Anthropic-compatible (optional, configuration only).** If you want
Messages-shaped semantics (for example a client or workflow built around
Anthropic's request/response shape), add a named provider profile instead:

```toml
[providers.neuralwatt]
type = "anthropic-compatible"
base_url = "https://api.neuralwatt.com/v1"
auth = "header"
auth_header = "x-api-key"
api_key_env = "NEURALWATT_API_KEY"
default_model = "glm-5.3"
```

Then select it with `--provider-profile neuralwatt` (or
`JCODE_NAMED_PROVIDER_PROFILE=neuralwatt`).

Notes on the Messages form:

- jcode's Anthropic transport appends `/messages` to the configured base, so
  the base must include `/v1` (`https://api.neuralwatt.com/v1`) even though
  Neuralwatt's own docs describe the Anthropic base as the bare host for
  Anthropic SDKs.
- `auth_header = "x-api-key"` is required; the default bearer header is not
  what the Messages surface expects.
- This entry is a named provider profile, **not** the `-p neuralwatt` preset.
  `-p neuralwatt` always selects the OpenAI-compatible preset; a named profile
  must be selected by name.

Both surfaces share the same key and the same upstream account, so picking one
does not change billing identity.

## Migrating from the generic profile

An existing `openai-compatible.env` pointed at Neuralwatt keeps working
unchanged. To adopt the first-class provider:

1. `jcode login --provider neuralwatt` (one-time credential move).
2. Use `-p neuralwatt` (or `/model`) from then on.

Nothing is removed, and the generic profile remains available for other
endpoints.

## Diagnosing

```bash
jcode provider-doctor neuralwatt --tier offline   # wiring only, no key/network
jcode provider-doctor neuralwatt --tier catalog   # also fetches /v1/models
```

`provider-doctor` resolves `neuralwatt` through the same preset registry, so it
walks Neuralwatt's credential, endpoint, and catalog checkpoints like any other
OpenAI-compatible provider.
