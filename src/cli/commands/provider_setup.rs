use anyhow::Result;
use std::io::Read;

use crate::cli::args::ProviderAuthArg;
use crate::config::NamedProviderAuth;
use crate::provider_catalog::{ProviderProfileReport, ProviderProfileSetup};

#[derive(Debug)]
pub(crate) struct ProviderAddOptions {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub context_window: Option<usize>,
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub api_key_stdin: bool,
    pub no_api_key: bool,
    pub auth: Option<ProviderAuthArg>,
    pub auth_header: Option<String>,
    pub env_file: Option<String>,
    pub set_default: bool,
    pub overwrite: bool,
    pub provider_routing: bool,
    pub model_catalog: bool,
    pub json: bool,
}

pub(crate) fn run_provider_add_command(options: ProviderAddOptions) -> Result<()> {
    let emit_json = options.json;
    let report = configure_provider_profile(options)?;

    if emit_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Added provider profile '{}'", report.profile);
        println!("  config: {}", report.config_path);
        println!("  base:   {}", report.api_base);
        println!("  model:  {}", report.model);
        println!("  auth:   {}", report.auth);
        if let Some(env_file_path) = &report.env_file_path {
            if report.api_key_stored {
                println!(
                    "  key:    {} in {}",
                    report.api_key_env.as_deref().unwrap_or("API key"),
                    env_file_path
                );
            } else {
                println!(
                    "  key:    {} (also reads {})",
                    report.api_key_env.as_deref().unwrap_or("API key"),
                    env_file_path
                );
            }
        } else if let Some(api_key_env) = &report.api_key_env {
            println!("  key:    environment variable {}", api_key_env);
        }
        if report.default_set {
            println!("  default: yes");
        }
        println!();
        println!("Run:       {}", report.run_command);
        println!("Validate:  {}", report.auth_test_command);
    }

    Ok(())
}



#[derive(Debug)]
pub(crate) struct ProviderRemoveOptions {
    pub name: String,
    pub json: bool,
}

pub(crate) fn run_provider_remove_command(options: ProviderRemoveOptions) -> Result<()> {
    let report = crate::provider_catalog::remove_provider_profile(&options.name)?;
    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Removed provider profile '{}'", report.profile);
        println!("  config: {}", report.config_path);
        match &report.env_file_path {
            Some(path) if report.removed_api_key_file => println!("  key:    removed {path}"),
            Some(path) => println!("  key:    {path} (not present)"),
            None => println!("  key:    none stored"),
        }
    }
    Ok(())
}

/// Convert CLI options into the shared setup call. Reading stdin happens here
/// so the daemon can pass a key value directly.
fn configure_provider_profile(options: ProviderAddOptions) -> Result<ProviderProfileReport> {
    let api_key = if options.api_key_stdin {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let key = input.trim().to_string();
        if key.is_empty() {
            anyhow::bail!("--api-key-stdin was set, but stdin was empty");
        }
        Some(key)
    } else {
        options
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(ToString::to_string)
    };
    let auth = options.auth.map(|auth| match auth {
        ProviderAuthArg::Bearer => NamedProviderAuth::Bearer,
        ProviderAuthArg::ApiKey => NamedProviderAuth::Header,
        ProviderAuthArg::None => NamedProviderAuth::None,
    });
    crate::provider_catalog::configure_provider_profile(ProviderProfileSetup {
        name: options.name,
        base_url: options.base_url,
        model: options.model,
        context_window: options.context_window,
        api_key_env: options.api_key_env,
        api_key,
        no_api_key: options.no_api_key,
        auth,
        auth_header: options.auth_header,
        env_file: options.env_file,
        set_default: options.set_default,
        overwrite: options.overwrite,
        provider_routing: options.provider_routing,
        model_catalog: options.model_catalog,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::ffi::OsString;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            crate::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            crate::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                crate::env::set_var(self.key, previous);
            } else {
                crate::env::remove_var(self.key);
            }
        }
    }

    fn base_options() -> ProviderAddOptions {
        ProviderAddOptions {
            name: "my-api".to_string(),
            base_url: "https://llm.example.com/v1".to_string(),
            model: "model-a".to_string(),
            context_window: Some(128_000),
            api_key_env: None,
            api_key: Some("secret-test-key".to_string()),
            api_key_stdin: false,
            no_api_key: false,
            auth: None,
            auth_header: None,
            env_file: None,
            set_default: true,
            overwrite: false,
            provider_routing: false,
            model_catalog: false,
            json: false,
        }
    }

    #[test]
    fn provider_add_writes_named_profile_env_file_and_default() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::TempDir::new().expect("temp dir");
        let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
        let _key = EnvVarGuard::remove("JCODE_PROVIDER_MY_API_API_KEY");
        let config_path = temp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "# keep this comment\n[provider]\nopenai_reasoning_effort = \"low\"\n",
        )
        .expect("write config");

        let report = configure_provider_profile(base_options()).expect("configure provider");

        assert_eq!(report.profile, "my-api");
        let config = std::fs::read_to_string(&config_path).expect("read config");
        assert!(config.contains("# keep this comment"));
        assert!(config.contains("default_provider = \"my-api\""));
        assert!(config.contains("default_model = \"model-a\""));
        assert!(config.contains("[providers.my-api]"));
        assert!(!config.contains("secret-test-key"));

        let parsed: Config = toml::from_str(&config).expect("valid config");
        assert_eq!(parsed.provider.default_provider.as_deref(), Some("my-api"));
        assert_eq!(parsed.provider.default_model.as_deref(), Some("model-a"));
        let profile = parsed.providers.get("my-api").expect("profile");
        assert_eq!(profile.base_url, "https://llm.example.com/v1");
        assert_eq!(profile.default_model.as_deref(), Some("model-a"));
        assert_eq!(
            profile.api_key_env.as_deref(),
            Some("JCODE_PROVIDER_MY_API_API_KEY")
        );
        assert_eq!(profile.env_file.as_deref(), Some("provider-my-api.env"));
        assert_eq!(profile.models[0].context_window, Some(128_000));

        let env_file = temp
            .path()
            .join("config")
            .join("jcode")
            .join("provider-my-api.env");
        let env_content = std::fs::read_to_string(env_file).expect("env file");
        assert!(env_content.contains("JCODE_PROVIDER_MY_API_API_KEY=secret-test-key"));
    }

    #[test]
    fn provider_add_rejects_remote_without_api_key_source() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::TempDir::new().expect("temp dir");
        let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
        let mut options = base_options();
        options.api_key = None;
        options.set_default = false;

        let err = configure_provider_profile(options).expect_err("should require key source");
        assert!(err.to_string().contains("needs an API key source"));
    }

    #[test]
    fn provider_add_allows_localhost_without_api_key() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::TempDir::new().expect("temp dir");
        let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
        let mut options = base_options();
        options.base_url = "http://localhost:8000/v1".to_string();
        options.api_key = None;
        options.set_default = false;

        configure_provider_profile(options).expect("localhost no-auth should work");
        let config = std::fs::read_to_string(temp.path().join("config.toml")).expect("config");
        assert!(config.contains("auth = \"none\""));
        assert!(config.contains("requires_api_key = false"));
    }

    #[test]
    fn provider_remove_deletes_profile_key_and_default() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::TempDir::new().expect("temp dir");
        let _home = EnvVarGuard::set("JCODE_HOME", temp.path());

        configure_provider_profile(base_options()).expect("configure provider");
        run_provider_remove_command(ProviderRemoveOptions {
            name: "my-api".to_string(),
            json: true,
        })
        .expect("remove provider");

        let config = std::fs::read_to_string(temp.path().join("config.toml")).expect("config");
        assert!(!config.contains("[providers.my-api]"));
        assert!(!config.contains("default_provider"));
        assert!(
            !temp
                .path()
                .join("config")
                .join("jcode")
                .join("provider-my-api.env")
                .exists()
        );
    }

    #[test]
    fn provider_remove_refuses_unknown_and_builtin_profiles() {
        let _lock = crate::storage::lock_test_env();
        let temp = tempfile::TempDir::new().expect("temp dir");
        let _home = EnvVarGuard::set("JCODE_HOME", temp.path());
        std::fs::write(temp.path().join("config.toml"), "[provider]\n").expect("config");

        let unknown = run_provider_remove_command(ProviderRemoveOptions {
            name: "missing".to_string(),
            json: false,
        })
        .expect_err("unknown profile");
        assert!(unknown.to_string().contains("No provider profile"), "{unknown}");

        let builtin = run_provider_remove_command(ProviderRemoveOptions {
            name: "openai".to_string(),
            json: false,
        })
        .expect_err("built-in profile");
        assert!(builtin.to_string().contains("built-in"), "{builtin}");
    }
}
