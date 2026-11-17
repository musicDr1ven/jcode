//! Create and remove named OpenAI-compatible provider profiles.
//!
//! Shared by the CLI (`jcode provider add`/`remove`) and the harness bridge so
//! both write the same config and env-file shape, validate the same names, and
//! refuse to touch built-in providers. Deliberately free of the application
//! foundation so the bridge binary stays small.

use anyhow::{Context as _, Result};
use jcode_config_types::{
    NamedProviderAuth, NamedProviderConfig, NamedProviderModelConfig, NamedProviderType,
};
use jcode_provider_env::save_named_api_key;
use jcode_provider_metadata::{
    api_base_uses_localhost, is_safe_env_file_name, is_safe_env_key_name, normalize_api_base,
    resolve_login_provider,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The subset of `config.toml` needed to create or remove a provider profile.
#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    providers: BTreeMap<String, NamedProviderConfig>,
    #[serde(default)]
    provider: ProviderDefaults,
}

#[derive(Debug, Default, Deserialize)]
struct ProviderDefaults {
    default_provider: Option<String>,
    default_model: Option<String>,
}

/// `$JCODE_HOME/config.toml`, the runtime's provider configuration file.
fn config_path() -> Result<PathBuf> {
    Ok(jcode_storage::jcode_dir()?.join("config.toml"))
}

/// Credential file binding for a user-created `[providers.<name>]` profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedProviderCredential {
    pub name: String,
    pub api_key_env: String,
    pub env_file: String,
}

/// Resolve a named profile's credential location without loading the full
/// runtime config cache. Returns `None` for unknown or unsafe profiles.
pub fn named_provider_profile(name: &str) -> Option<NamedProviderCredential> {
    let content = std::fs::read_to_string(config_path().ok()?).ok()?;
    let config = parse_config_or_default(&content).ok()?;
    let profile = config.providers.get(name)?;
    let api_key_env = profile.api_key_env.clone()?;
    let env_file = profile.env_file.clone()?;
    if !is_safe_env_key_name(&api_key_env) || !is_safe_env_file_name(&env_file) {
        return None;
    }
    Some(NamedProviderCredential {
        name: name.to_string(),
        api_key_env,
        env_file,
    })
}

/// Options for creating or replacing a named OpenAI-compatible provider profile.
#[derive(Debug, Clone)]
pub struct ProviderProfileSetup {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub context_window: Option<usize>,
    pub api_key_env: Option<String>,
    /// API key value to store. Callers reading a key from stdin must resolve it
    /// before calling so the daemon can pass a value directly.
    pub api_key: Option<String>,
    pub no_api_key: bool,
    /// Explicit auth mode. `None` means bearer auth, or no auth for localhost.
    pub auth: Option<NamedProviderAuth>,
    pub auth_header: Option<String>,
    pub env_file: Option<String>,
    pub set_default: bool,
    pub overwrite: bool,
    pub provider_routing: bool,
    pub model_catalog: bool,
}

/// Outcome of creating or replacing a provider profile.
#[derive(Debug, Serialize)]
pub struct ProviderProfileReport {
    pub status: &'static str,
    pub profile: String,
    pub config_path: String,
    pub api_base: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub env_file: Option<String>,
    pub env_file_path: Option<String>,
    pub api_key_stored: bool,
    pub auth: String,
    pub default_set: bool,
    pub run_command: String,
    pub auth_test_command: String,
}

pub fn configure_provider_profile(
    options: ProviderProfileSetup,
) -> Result<ProviderProfileReport> {
    let name = validate_profile_name(&options.name)?;
    ensure_profile_name_not_reserved(&name)?;

    let api_base = normalize_api_base(&options.base_url).ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid --base-url '{}'. Use https://... or http://localhost/127.0.0.1/private-LAN for local servers.",
            options.base_url
        )
    })?;
    let model = options.model.trim().to_string();
    if model.is_empty() {
        anyhow::bail!("--model cannot be empty");
    }
    if matches!(options.context_window, Some(0)) {
        anyhow::bail!("--context-window must be greater than 0");
    }

    let api_key = options.api_key.clone();
    let auth = resolve_auth_mode(&options, api_key.as_deref(), &api_base)?;
    let uses_auth = !matches!(auth, NamedProviderAuth::None);

    if !uses_auth && options.auth_header.is_some() {
        anyhow::bail!("--auth-header can only be used with --auth api-key");
    }
    if !matches!(auth, NamedProviderAuth::Header) && options.auth_header.is_some() {
        anyhow::bail!("--auth-header requires --auth api-key");
    }

    let api_key_env = if uses_auth {
        Some(resolve_api_key_env(&name, options.api_key_env.as_deref())?)
    } else {
        None
    };

    let env_file = if uses_auth && (api_key.is_some() || options.env_file.is_some()) {
        Some(resolve_env_file(&name, options.env_file.as_deref())?)
    } else {
        None
    };

    if uses_auth
        && api_key.is_none()
        && options.api_key_env.is_none()
        && options.env_file.is_none()
        && !api_base_uses_localhost(&api_base)
    {
        anyhow::bail!(
            "Remote provider '{}' needs an API key source. Use --api-key-env NAME, --api-key-stdin, --api-key VALUE, or --auth none if this endpoint truly needs no auth.",
            name
        );
    }

    if let (Some(key), Some(env_key), Some(file_name)) = (
        api_key.as_deref(),
        api_key_env.as_deref(),
        env_file.as_deref(),
    ) {
        save_named_api_key(file_name, env_key, key)?;
    }
    let api_key_stored = api_key.is_some() && env_file.is_some();

    let profile = NamedProviderConfig {
        provider_type: NamedProviderType::OpenAiCompatible,
        base_url: api_base.clone(),
        api: None,
        auth: auth.clone(),
        auth_header: match auth {
            NamedProviderAuth::Header => options
                .auth_header
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            _ => None,
        },
        headers: std::collections::BTreeMap::new(),
        api_key_env: api_key_env.clone(),
        api_key: None,
        env_file: env_file.clone(),
        default_model: Some(model.clone()),
        requires_api_key: Some(uses_auth),
        provider_routing: options.provider_routing,
        model_catalog: options.model_catalog,
        allow_provider_pinning: options.provider_routing,
        models: vec![NamedProviderModelConfig {
            id: model.clone(),
            reasoning: None,
            reasoning_effort: None,
            context_window: options.context_window,
            input: Vec::new(),
        }],
        extra_body: None,
        supports_reasoning_effort: None,
        disable_reasoning_heuristics: false,
    };

    let config_path = config_path()?;
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let existing = parse_config_or_default(&content).with_context(|| {
        format!(
            "failed to parse existing config at {}",
            config_path.display()
        )
    })?;
    if existing.providers.contains_key(&name) && !options.overwrite {
        anyhow::bail!(
            "Provider profile '{}' already exists. Re-run with --overwrite to replace it.",
            name
        );
    }

    let mut updated = if options.overwrite {
        remove_named_provider_sections(&content, &name)
    } else {
        content
    };
    if options.set_default {
        updated = upsert_provider_defaults(updated, &name, &model);
    }
    updated = append_profile_section(updated, &name, &profile);

    toml::from_str::<toml::Value>(&updated).with_context(|| {
        format!(
            "generated provider config for '{}' was not valid TOML",
            name
        )
    })?;

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, updated)?;

    let env_file_path = env_file
        .as_deref()
        .map(|file| jcode_storage::app_config_dir().map(|dir| dir.join(file)))
        .transpose()?
        .map(path_to_string);

    Ok(ProviderProfileReport {
        status: "ok",
        profile: name.clone(),
        config_path: path_to_string(config_path),
        api_base,
        model: model.clone(),
        api_key_env,
        env_file,
        env_file_path,
        api_key_stored,
        auth: auth_label(&auth).to_string(),
        default_set: options.set_default,
        run_command: format!(
            "jcode --provider-profile {} --model {} run 'hello'",
            shell_quote(&name),
            shell_quote(&model)
        ),
        auth_test_command: format!(
            "jcode --provider-profile {} auth-test --prompt {}",
            shell_quote(&name),
            shell_quote("Reply exactly JCODE_PROVIDER_SETUP_OK")
        ),
    })
}

fn parse_config_or_default(content: &str) -> Result<ConfigFile> {
    if content.trim().is_empty() {
        Ok(ConfigFile::default())
    } else {
        Ok(toml::from_str::<ConfigFile>(content)?)
    }
}

fn validate_profile_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        anyhow::bail!("provider profile name cannot be empty");
    }
    if name.len() > 64 {
        anyhow::bail!("provider profile name must be at most 64 characters");
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        anyhow::bail!("provider profile name cannot be empty");
    };
    if !first.is_ascii_alphanumeric() {
        anyhow::bail!("provider profile name must start with a letter or number");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("provider profile name may only contain ASCII letters, numbers, '-' and '_'");
    }
    Ok(name.to_string())
}

fn ensure_profile_name_not_reserved(name: &str) -> Result<()> {
    const RESERVED_PROVIDER_NAMES: &[&str] = &[
        "auto",
        "claude-subprocess",
        "compat",
        "custom",
        "azure-openai",
        "aoai",
    ];
    if resolve_login_provider(name).is_some()
        || RESERVED_PROVIDER_NAMES
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        anyhow::bail!(
            "'{}' is a built-in provider id or alias. Choose a non-reserved profile name such as '{}-api'.",
            name,
            name
        );
    }
    Ok(())
}

fn resolve_auth_mode(
    options: &ProviderProfileSetup,
    api_key: Option<&str>,
    api_base: &str,
) -> Result<NamedProviderAuth> {
    if options.no_api_key {
        return Ok(NamedProviderAuth::None);
    }
    if matches!(options.auth, Some(NamedProviderAuth::None)) {
        if api_key.is_some() || options.api_key_env.is_some() || options.env_file.is_some() {
            anyhow::bail!("--auth none cannot be combined with API key options");
        }
        return Ok(NamedProviderAuth::None);
    }
    if options.auth.is_none()
        && api_key.is_none()
        && options.api_key_env.is_none()
        && options.env_file.is_none()
        && api_base_uses_localhost(api_base)
    {
        return Ok(NamedProviderAuth::None);
    }

    Ok(options.auth.clone().unwrap_or(NamedProviderAuth::Bearer))
}

fn resolve_api_key_env(name: &str, configured: Option<&str>) -> Result<String> {
    let env_name = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| derived_api_key_env(name));
    if !is_safe_env_key_name(&env_name) {
        anyhow::bail!(
            "Invalid --api-key-env '{}'. Use uppercase letters, numbers, and underscores only.",
            env_name
        );
    }
    Ok(env_name)
}

fn resolve_env_file(name: &str, configured: Option<&str>) -> Result<String> {
    let file = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("provider-{}.env", name));
    if !is_safe_env_file_name(&file) {
        anyhow::bail!(
            "Invalid --env-file '{}'. Use a file name only, with letters, numbers, '.', '_' or '-'.",
            file
        );
    }
    Ok(file)
}

fn derived_api_key_env(name: &str) -> String {
    let suffix = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("JCODE_PROVIDER_{}_API_KEY", suffix)
}

fn append_profile_section(
    mut content: String,
    name: &str,
    profile: &NamedProviderConfig,
) -> String {
    if !content.trim().is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if !content.is_empty() && !content.ends_with("\n\n") {
        content.push('\n');
    }

    content.push_str(&format!("[providers.{name}]\n"));
    content.push_str("type = \"openai-compatible\"\n");
    content.push_str(&format!("base_url = {}\n", toml_quote(&profile.base_url)));
    content.push_str(&format!(
        "auth = {}\n",
        toml_quote(auth_label(&profile.auth))
    ));
    if let Some(header) = profile.auth_header.as_deref() {
        content.push_str(&format!("auth_header = {}\n", toml_quote(header)));
    }
    if let Some(api_key_env) = profile.api_key_env.as_deref() {
        content.push_str(&format!("api_key_env = {}\n", toml_quote(api_key_env)));
    }
    if let Some(env_file) = profile.env_file.as_deref() {
        content.push_str(&format!("env_file = {}\n", toml_quote(env_file)));
    }
    if let Some(default_model) = profile.default_model.as_deref() {
        content.push_str(&format!("default_model = {}\n", toml_quote(default_model)));
    }
    if let Some(requires_api_key) = profile.requires_api_key {
        content.push_str(&format!("requires_api_key = {requires_api_key}\n"));
    }
    if profile.provider_routing {
        content.push_str("provider_routing = true\nallow_provider_pinning = true\n");
    }
    if profile.model_catalog {
        content.push_str("model_catalog = true\n");
    }

    for model in &profile.models {
        content.push_str(&format!("\n[[providers.{name}.models]]\n"));
        content.push_str(&format!("id = {}\n", toml_quote(&model.id)));
        if let Some(limit) = model.context_window {
            content.push_str(&format!("context_window = {limit}\n"));
        }
    }

    content
}

fn upsert_provider_defaults(content: String, profile: &str, model: &str) -> String {
    let mut lines = split_lines_lossy(&content);
    let provider_idx = lines.iter().position(|line| line.trim() == "[provider]");

    let Some(idx) = provider_idx else {
        let mut prefix = String::from("[provider]\n");
        prefix.push_str(&format!("default_provider = {}\n", toml_quote(profile)));
        prefix.push_str(&format!("default_model = {}\n\n", toml_quote(model)));
        if content.trim().is_empty() {
            return prefix;
        }
        return format!("{prefix}{}", content.trim_start_matches('\n'));
    };

    let end = lines
        .iter()
        .enumerate()
        .skip(idx + 1)
        .find(|(_, line)| is_toml_header(line))
        .map(|(line_idx, _)| line_idx)
        .unwrap_or(lines.len());

    upsert_key_in_range(
        &mut lines,
        idx + 1,
        end,
        "default_provider",
        &toml_quote(profile),
    );
    let end = lines
        .iter()
        .enumerate()
        .skip(idx + 1)
        .find(|(_, line)| is_toml_header(line))
        .map(|(line_idx, _)| line_idx)
        .unwrap_or(lines.len());
    upsert_key_in_range(
        &mut lines,
        idx + 1,
        end,
        "default_model",
        &toml_quote(model),
    );

    join_lines(lines)
}

fn upsert_key_in_range(lines: &mut Vec<String>, start: usize, end: usize, key: &str, value: &str) {
    for line in lines.iter_mut().take(end).skip(start) {
        if line_has_toml_key(line, key) {
            *line = format!("{key} = {value}");
            return;
        }
    }
    lines.insert(end, format!("{key} = {value}"));
}

fn remove_named_provider_sections(content: &str, name: &str) -> String {
    let lines = split_lines_lossy(content);
    let mut kept = Vec::with_capacity(lines.len());
    let mut skip = false;

    for line in lines {
        if is_toml_header(&line) {
            skip = is_named_provider_header(&line, name);
        }
        if !skip {
            kept.push(line);
        }
    }

    join_lines(kept)
}

fn is_named_provider_header(line: &str, name: &str) -> bool {
    let trimmed = line.trim();
    let inner = if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
        &trimmed[2..trimmed.len() - 2]
    } else if trimmed.starts_with('[') && trimmed.ends_with(']') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        return false;
    };
    let inner = inner.trim();
    let plain = format!("providers.{name}");
    let double_quoted = format!("providers.{}", toml_quote(name));
    let single_quoted = format!("providers.'{name}'");
    inner == plain
        || inner == format!("{plain}.models")
        || inner == double_quoted
        || inner == format!("{double_quoted}.models")
        || inner == single_quoted
        || inner == format!("{single_quoted}.models")
}

fn is_toml_header(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

fn line_has_toml_key(line: &str, key: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix(key) else {
        return false;
    };
    rest.trim_start().starts_with('=')
}

fn split_lines_lossy(content: &str) -> Vec<String> {
    content.lines().map(ToString::to_string).collect()
}

fn join_lines(lines: Vec<String>) -> String {
    let mut joined = lines.join("\n");
    if !joined.is_empty() {
        joined.push('\n');
    }
    joined
}

fn auth_label(auth: &NamedProviderAuth) -> &'static str {
    match auth {
        NamedProviderAuth::Bearer => "bearer",
        NamedProviderAuth::Header => "header",
        NamedProviderAuth::None => "none",
    }
}

fn toml_quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        eprintln!("failed to quote provider config string with serde_json: {error}");
        format!("\"{}\"", value.escape_default())
    })
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn path_to_string(path: PathBuf) -> String {
    path.display().to_string()
}

/// Outcome of removing a named provider profile.
#[derive(Debug, Serialize)]
pub struct ProviderProfileRemovalReport {
    pub profile: String,
    pub config_path: String,
    pub env_file_path: Option<String>,
    pub removed_api_key_file: bool,
}

/// Remove a user-created named provider profile and its stored API key.
///
/// Built-in provider ids and aliases are refused so a typo can never delete
/// runtime configuration that Desktop does not own.
pub fn remove_provider_profile(raw_name: &str) -> Result<ProviderProfileRemovalReport> {
    let name = validate_profile_name(raw_name)?;
    ensure_profile_name_not_reserved(&name)?;

    let config_path = config_path()?;
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read config at {}", config_path.display()))?;
    if content.trim().is_empty() {
        anyhow::bail!(
            "No provider profiles are configured at {}",
            config_path.display()
        );
    }

    let existing = parse_config_or_default(&content).with_context(|| {
        format!(
            "failed to parse existing config at {}",
            config_path.display()
        )
    })?;
    let profile = existing
        .providers
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("No provider profile named '{}'", name))?;

    let mut updated = remove_named_provider_sections(&content, &name);
    updated = clear_provider_defaults_pointing_at(updated, &name);
    toml::from_str::<toml::Value>(&updated).with_context(|| {
        format!(
            "generated provider config after removing '{}' was not valid TOML",
            name
        )
    })?;
    std::fs::write(&config_path, updated)?;

    let env_file_path = match profile.env_file.as_deref() {
        Some(file) if is_safe_env_file_name(file) => jcode_storage::app_config_dir()
            .ok()
            .map(|dir| dir.join(file)),
        _ => None,
    };
    let mut removed_api_key_file = false;
    if let Some(path) = env_file_path.as_deref()
        && path.is_file()
    {
        removed_api_key_file = std::fs::remove_file(path).is_ok();
    }

    Ok(ProviderProfileRemovalReport {
        profile: name,
        config_path: path_to_string(config_path),
        env_file_path: env_file_path
            .as_deref()
            .map(|path| path.display().to_string()),
        removed_api_key_file,
    })
}

/// Clear `[provider]` default keys when they still point at a removed profile.
fn clear_provider_defaults_pointing_at(content: String, name: &str) -> String {
    let lines = split_lines_lossy(&content);
    let Some(start) = lines.iter().position(|line| line.trim() == "[provider]") else {
        return content;
    };
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| is_toml_header(line))
        .map(|(index, _)| index)
        .unwrap_or(lines.len());

    let quoted = toml_quote(name);
    let points_here = lines[start + 1..end].iter().any(|line| {
        line_has_toml_key(line, "default_provider")
            && line
                .split_once('=')
                .map(|(_, value)| value.trim().trim_end_matches(',').trim() == quoted)
                .unwrap_or(false)
    });
    if !points_here {
        return content;
    }

    let mut kept = Vec::with_capacity(lines.len());
    for (index, line) in lines.into_iter().enumerate() {
        let in_section = index > start && index < end;
        if in_section
            && (line_has_toml_key(&line, "default_provider")
                || line_has_toml_key(&line, "default_model"))
        {
            continue;
        }
        kept.push(line);
    }
    join_lines(kept)
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            set_env(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                set_env(self.key, previous);
            } else {
                remove_env(self.key);
            }
        }
    }

    fn setup_options(name: &str) -> ProviderProfileSetup {
        ProviderProfileSetup {
            name: name.to_string(),
            base_url: "https://llm.example.com/v1".to_string(),
            model: "model-a".to_string(),
            context_window: Some(128_000),
            api_key_env: None,
            api_key: Some("secret-test-key".to_string()),
            no_api_key: false,
            auth: None,
            auth_header: None,
            env_file: None,
            set_default: true,
            overwrite: false,
            provider_routing: false,
            model_catalog: false,
        }
    }

    #[test]
    fn setup_writes_profile_key_and_default() {
        let _lock = env_lock();
        let temp = tempfile::TempDir::new().expect("temp dir");
        let _home = EnvVarGuard::set("JCODE_HOME", temp.path());

        let report = configure_provider_profile(setup_options("my-api")).expect("configure");

        assert_eq!(report.profile, "my-api");
        let config = std::fs::read_to_string(temp.path().join("config.toml")).expect("config");
        assert!(config.contains("[providers.my-api]"));
        assert!(config.contains("default_provider = \"my-api\""));
        let env_file = temp
            .path()
            .join("config")
            .join("jcode")
            .join("provider-my-api.env");
        assert!(std::fs::read_to_string(env_file)
            .expect("env file")
            .contains("JCODE_PROVIDER_MY_API_API_KEY=secret-test-key"));
    }

    #[test]
    fn remove_deletes_profile_key_and_default_but_refuses_builtin() {
        let _lock = env_lock();
        let temp = tempfile::TempDir::new().expect("temp dir");
        let _home = EnvVarGuard::set("JCODE_HOME", temp.path());

        configure_provider_profile(setup_options("my-api")).expect("configure");
        let report = remove_provider_profile("my-api").expect("remove");
        assert!(report.removed_api_key_file);

        let config = std::fs::read_to_string(temp.path().join("config.toml")).expect("config");
        assert!(!config.contains("[providers.my-api]"));
        assert!(!config.contains("default_provider"));
        let env_file = temp
            .path()
            .join("config")
            .join("jcode")
            .join("provider-my-api.env");
        assert!(!env_file.exists());

        let error = remove_provider_profile("openai").expect_err("built-in must be refused");
        assert!(error.to_string().contains("built-in"), "{error}");
    }
}

#[cfg(test)]
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
fn set_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: tests serialize environment access with `env_lock`.
    unsafe { std::env::set_var(key, value) }
}

#[cfg(test)]
fn remove_env(key: &str) {
    // SAFETY: tests serialize environment access with `env_lock`.
    unsafe { std::env::remove_var(key) }
}
