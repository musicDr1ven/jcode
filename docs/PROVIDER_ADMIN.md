# Provider administration

Jcode can add, credential, and remove named OpenAI-compatible provider
profiles without hand-editing `config.toml`. The CLI and the harness API share
one implementation (`crates/jcode-provider-setup`), so both write the same
shape and enforce the same validation.

## Add a named provider profile

```sh
printf '%s\n' "$MY_GATEWAY_KEY" | jcode provider add my-gateway \
  --base-url https://llm.example.com/v1 \
  --model my-model \
  --api-key-stdin
```

`--api-key-stdin` keeps the secret out of shell history and the process command
line. `--api-key` exists for compatibility but is discouraged for the same
reason. Omit the key entirely (or pass `--no-api-key`) for a localhost endpoint
that needs no auth.

Required: `name`, `--base-url` (`https://`, or `http://` for localhost, private
LAN, or bare-IP hosts), and `--model`.

Useful options:

| Option | Purpose |
| --- | --- |
| `--context-window N` | Declare the model's context window. |
| `--api-key-env NAME` | Env var that holds the key (default `JCODE_PROVIDER_<NAME>_API_KEY`). |
| `--env-file NAME` | Private env file under the app config dir (default `provider-<name>.env`). |
| `--auth bearer\|header\|none` | Auth style; `header` requires `--auth-header`. |
| `--set-default` | Also make this profile the startup default provider/model. |
| `--overwrite` | Replace an existing profile with the same name. |
| `--json` | Emit a secret-free machine-readable report. |

The profile is stored as:

```toml
[providers.my-gateway]
type = "openai-compatible"
base_url = "https://llm.example.com/v1"
auth = "bearer"
api_key_env = "MY_GATEWAY_API_KEY"
env_file = "provider-my-gateway.env"
default_model = "my-model"
requires_api_key = true

[[providers.my-gateway.models]]
id = "my-model"
context_window = 200000
```

The API key never enters `config.toml`. It is written to the owner-only env file
(`<app config dir>/provider-my-gateway.env`, mode `0600`) named by `env_file`.

## Replace or clear a credential

```sh
printf '%s\n' "$NEW_KEY" | jcode login my-gateway --api-key-stdin
jcode login my-gateway --api-key "$NEW_KEY"   # compatibility form
```

Harness clients can also provision keys with `set_api_key` / `clear_api_key`,
which now resolve any built-in API-key provider, any preset OpenAI-compatible
profile, and any named `[providers.<name>]` profile.

## Remove a named profile

```sh
jcode provider remove my-gateway
jcode provider remove my-gateway --json
```

Removal deletes the `[providers.<name>]` sections, drops a `[provider]` default
that pointed at the profile, and deletes its stored env file. Built-in provider
ids and aliases are refused, so a typo cannot delete runtime configuration Jcode
does not own.

## Harness API

Runtimes that accept these operations advertise the `provider_admin` capability:

| Request | Purpose |
| --- | --- |
| `provider_add` | Create or replace a named profile (same fields as the CLI, `auth` as `bearer`/`header`/`none`). |
| `provider_remove` | Remove a user-created named profile. |
| `set_api_key` / `clear_api_key` | Store or clear a provider credential; built-ins, presets, and named profiles resolve to their env file. |

SDK methods: `JcodeClient.add_provider_profile`, `remove_provider_profile`,
`set_api_key`, `clear_api_key` (TypeScript: `addProviderProfile`,
`removeProviderProfile`, `setApiKey`, `clearApiKey`). Clients that do not see
`provider_admin` should fall back to the CLI instructions above.
