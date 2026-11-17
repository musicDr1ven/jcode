//! Secure credential binding and atomic owner-only persistence.

use super::*;
use jcode_provider_metadata::{
    is_safe_env_file_name, is_safe_env_key_name, openai_compatible_profiles,
};

/// Where a provider's API key lives: the canonical provider id plus the env
/// keys and file that `jcode auth status` reads back.
pub(super) struct CredentialBinding {
    pub provider: String,
    pub env_keys: Vec<String>,
    pub file_name: String,
}

impl BridgeState {
    /// Resolve an API-key provider to its credential file.
    ///
    /// Built-in ids keep their historical env keys (Gemini owns two). Named
    /// `[providers.<name>]` profiles resolve from the runtime config, and
    /// preset OpenAI-compatible profiles resolve from provider metadata.
    pub(super) fn credential_binding(provider: &str) -> Option<CredentialBinding> {
        let raw = provider.trim();
        if raw.is_empty() {
            return None;
        }
        let normalized = raw.to_ascii_lowercase();
        if let Some(binding) = Self::builtin_credential_binding(&normalized) {
            return Some(binding);
        }

        let profile_name = raw.strip_prefix("openai-compatible:").unwrap_or(raw);
        if let Some(profile) = jcode_provider_setup::named_provider_profile(profile_name) {
            return Some(CredentialBinding {
                provider: profile.name,
                env_keys: vec![profile.api_key_env],
                file_name: profile.env_file,
            });
        }

        let preset = openai_compatible_profiles()
            .iter()
            .find(|profile| profile.id == profile_name)?;
        if !is_safe_env_key_name(preset.api_key_env) || !is_safe_env_file_name(preset.env_file) {
            return None;
        }
        Some(CredentialBinding {
            provider: preset.id.to_string(),
            env_keys: vec![preset.api_key_env.to_string()],
            file_name: preset.env_file.to_string(),
        })
    }

    /// Canonical built-in ids whose credential location predates the metadata
    /// catalog. Kept exact so alias normalization and Gemini's two env keys do
    /// not change.
    fn builtin_credential_binding(provider: &str) -> Option<CredentialBinding> {
        let (id, env_keys, file_name): (&str, &[&str], &str) = match provider {
            "claude-api" | "anthropic-api" | "anthropic-key" | "claude-key" => {
                ("claude-api", &["ANTHROPIC_API_KEY"], "anthropic.env")
            }
            "openai-api" | "openai-key" | "openai-apikey" => {
                ("openai-api", &["OPENAI_API_KEY"], "openai.env")
            }
            "openrouter" => ("openrouter", &["OPENROUTER_API_KEY"], "openrouter.env"),
            "cursor" => ("cursor", &["CURSOR_API_KEY"], "cursor.env"),
            "gemini" | "gemini-api" | "google-gemini" => (
                "gemini",
                &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
                "gemini.env",
            ),
            "jcode" | "subscription" | "jcode-subscription" => {
                ("jcode", &["JCODE_API_KEY"], "jcode-subscription.env")
            }
            _ => return None,
        };
        Some(CredentialBinding {
            provider: id.to_string(),
            env_keys: env_keys.iter().map(|key| (*key).to_string()).collect(),
            file_name: file_name.to_string(),
        })
    }

    pub(super) fn write_credential(
        file_name: &str,
        env_keys: &[String],
        value: Option<&str>,
    ) -> Result<(), String> {
        let _write_guard = Self::state_write_guard();
        let dir = Self::app_config_dir()
            .ok_or_else(|| "could not resolve jcode config directory".to_string())?;
        Self::ensure_owner_only_dir(&dir)?;
        let path = dir.join(file_name);
        Self::validate_owner_file_if_exists(&path)?;
        let existing = match std::fs::read_to_string(&path) {
            Ok(existing) => existing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(format!("read credential file: {error}")),
        };
        let mut lines: Vec<String> = existing
            .lines()
            .filter(|line| {
                line.split_once('=')
                    .is_none_or(|(key, _)| !env_keys.iter().any(|candidate| candidate == key.trim()))
            })
            .map(str::to_string)
            .collect();
        if let Some(value) = value {
            if value.contains(['\n', '\r', '\0']) || value.trim() != value {
                return Err("API key must be a trimmed, non-NUL single line".into());
            }
            lines.push(format!("{}={value}", env_keys[0]));
        }
        let mut content = lines.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        Self::atomic_owner_write(&path, content.as_bytes())
    }

    pub(super) fn atomic_owner_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "owner-only write path has no parent".to_string())?;
        Self::ensure_owner_only_dir(parent)?;
        Self::validate_owner_file_if_exists(path)?;
        let temp = path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("credential write clock is before Unix epoch: {error}"))?
                .as_nanos()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|error| error.to_string())?;
        let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write_result {
            if let Err(cleanup_error) = std::fs::remove_file(&temp)
                && cleanup_error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(format!(
                    "credential write failed: {error}; temporary-file cleanup failed: {cleanup_error}"
                ));
            }
            return Err(error.to_string());
        }
        Self::atomic_replace(&temp, path).map_err(|error| match std::fs::remove_file(&temp) {
            Ok(()) => error.to_string(),
            Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => {
                error.to_string()
            }
            Err(cleanup_error) => format!(
                "credential replace failed: {error}; temporary-file cleanup failed: {cleanup_error}"
            ),
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("sync owner-only directory: {error}"))?;
        Ok(())
    }

    pub(super) fn ensure_owner_only_dir(path: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(path).map_err(|error| error.to_string())?;
        let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "{} must be a non-symlink directory",
                path.display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err(format!(
                    "{} is not owned by the current user",
                    path.display()
                ));
            }
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn validate_owner_file_if_exists(path: &std::path::Path) -> Result<(), String> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "{} must be a regular, non-symlink file",
                path.display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err(format!(
                    "{} is not owned by the current user",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(windows))]
    pub(super) fn atomic_replace(
        from: &std::path::Path,
        to: &std::path::Path,
    ) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    #[cfg(windows)]
    pub(super) fn atomic_replace(
        from: &std::path::Path,
        to: &std::path::Path,
    ) -> std::io::Result<()> {
        use std::os::windows::ffi::OsStrExt;
        unsafe extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        let replaced = unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
