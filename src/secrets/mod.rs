//! API key resolution with a secure, layered fallback chain.
//!
//! Resolution order for [`KeySource::Auto`]:
//! 1. systemd `$CREDENTIALS_DIRECTORY` (`LoadCredential=`)
//! 2. Freedesktop Secret Service (GNOME Keyring / KWallet)
//! 3. A `0600` file in `~/.config/ttsandsttp/`
//! 4. The environment variable (development only)
//!
//! Keys are scoped per provider id. The resolved key is wrapped in
//! [`SecretString`] (zeroized on drop) and is never logged.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use secrecy::{ExposeSecret, SecretString};
use secret_service::{EncryptionType, SecretService};

use crate::config::{ApiConfig, KeySource};

/// Provider-agnostic description of where one provider's API key lives.
#[derive(Debug, Clone)]
pub struct KeySpec {
    /// Human-facing provider label used in prompts and errors.
    pub label: String,
    /// Provider id used to scope credential/keyring/file names.
    pub id: String,
    /// systemd credential names to try, in order.
    pub credentials: Vec<String>,
    /// Secret Service `account` attribute.
    pub keyring_account: String,
    /// Default `0600` key file path (before user override).
    pub key_file: PathBuf,
    /// Environment variable fallback.
    pub env: String,
    /// Resolution strategy.
    pub key_source: KeySource,
}

impl KeySpec {
    /// DashScope-compatible spec preserving the historical `dashscope-<region>`
    /// credential / `dashscope.<region>.key` file / `dashscope/<region>` keyring
    /// naming.
    pub fn dashscope(api: &ApiConfig) -> Self {
        let slug = api.region.slug();
        Self {
            label: "DashScope".to_string(),
            id: "dashscope".to_string(),
            credentials: vec![format!("dashscope-{}", slug), "dashscope".to_string()],
            keyring_account: format!("dashscope/{}", slug),
            key_file: if !api.key_file.is_empty() {
                expand_tilde(&api.key_file)
            } else {
                config_dir().join(format!("dashscope.{}.key", slug))
            },
            env: api.effective_env(),
            key_source: api.key_source,
        }
    }

    /// Spec for any catalog or user-defined provider.
    pub fn provider(
        id: &str,
        label: &str,
        catalog_env: &str,
        env_override: &str,
        key_file: &str,
        key_source: KeySource,
    ) -> Self {
        let env = if !env_override.trim().is_empty() {
            env_override.trim().to_string()
        } else if !catalog_env.trim().is_empty() {
            catalog_env.trim().to_string()
        } else {
            format!("{}_API_KEY", id.to_uppercase().replace('-', "_"))
        };
        Self {
            label: if label.is_empty() { id } else { label }.to_string(),
            id: id.to_string(),
            credentials: vec![id.to_string()],
            keyring_account: id.to_string(),
            key_file: if !key_file.is_empty() {
                expand_tilde(key_file)
            } else {
                config_dir().join(format!("{}.key", id))
            },
            env,
            key_source,
        }
    }
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("ttsandsttp")
}

/// Resolved secret storage helpers
pub struct Secrets;

impl Secrets {
    /// Resolve the DashScope API key (legacy entry point).
    pub async fn resolve(api: &ApiConfig) -> Result<SecretString> {
        Self::resolve_spec(&KeySpec::dashscope(api)).await
    }

    /// Resolve the API key for a provider according to its [`KeySource`].
    pub async fn resolve_spec(spec: &KeySpec) -> Result<SecretString> {
        match spec.key_source {
            KeySource::Auto => {
                if let Some(key) = from_credential(spec)? {
                    tracing::debug!("{} API key resolved from systemd credentials", spec.label);
                    return Ok(key);
                }
                match from_keyring(spec).await {
                    Ok(Some(key)) => {
                        tracing::debug!(
                            "{} API key resolved from Secret Service keyring",
                            spec.label
                        );
                        return Ok(key);
                    }
                    Ok(None) => {}
                    Err(e) => tracing::debug!("Secret Service lookup unavailable: {}", e),
                }
                if let Some(key) = from_file(spec)? {
                    tracing::debug!("{} API key resolved from key file", spec.label);
                    return Ok(key);
                }
                if let Some(key) = from_env(spec) {
                    tracing::warn!(
                        "{} API key resolved from environment variable ({}); \
                         prefer `ttsandsttp set-key {}` for secure storage",
                        spec.label,
                        spec.env,
                        spec.id
                    );
                    return Ok(key);
                }
                bail!(
                    "No API key found for {} (provider '{}'). Store one with \
                     `ttsandsttp set-key {}`, set it via systemd LoadCredential, or export {}",
                    spec.label,
                    spec.id,
                    spec.id,
                    spec.env
                )
            }
            KeySource::Credential => from_credential(spec)?.context(format!(
                "No {} API key in $CREDENTIALS_DIRECTORY (add LoadCredential={}:<path>)",
                spec.label,
                spec.credentials.first().map(|s| s.as_str()).unwrap_or(&spec.id)
            )),
            KeySource::Keyring => from_keyring(spec).await?.context(format!(
                "No {} API key in the Secret Service keyring (run `ttsandsttp set-key {}`)",
                spec.label, spec.id
            )),
            KeySource::File => from_file(spec)?.context(format!(
                "No {} API key file at {}",
                spec.label,
                spec.key_file.display()
            )),
            KeySource::Env => from_env(spec).context(format!(
                "Environment variable {} is not set",
                spec.env
            )),
        }
    }

    /// Store the DashScope API key in the Secret Service keyring.
    pub async fn store(api: &ApiConfig, key: &str) -> Result<()> {
        Self::store_spec(&KeySpec::dashscope(api), key).await
    }

    /// Store a provider's API key in the Secret Service keyring, replacing
    /// any existing entry.
    pub async fn store_spec(spec: &KeySpec, key: &str) -> Result<()> {
        let key = key.trim();
        if key.is_empty() {
            bail!("Refusing to store an empty API key");
        }

        let account = spec.keyring_account.clone();
        let ss = connect_keyring().await?;
        let collection = ss
            .get_default_collection()
            .await
            .context("Failed to open the default keyring collection")?;
        collection
            .ensure_unlocked()
            .await
            .context("Failed to unlock the keyring")?;

        let attributes =
            HashMap::from([("service", "ttsandsttp"), ("account", account.as_str())]);
        collection
            .create_item(
                &format!("ttsandsttp {} API key ({})", spec.label, spec.id),
                attributes,
                key.as_bytes(),
                true,
                "text/plain",
            )
            .await
            .context("Failed to store the key in the keyring")?;

        Ok(())
    }

    /// Remove the DashScope API key from the Secret Service keyring.
    pub async fn delete(api: &ApiConfig) -> Result<()> {
        Self::delete_spec(&KeySpec::dashscope(api)).await
    }

    /// Remove a provider's API key from the Secret Service keyring.
    pub async fn delete_spec(spec: &KeySpec) -> Result<()> {
        let ss = connect_keyring().await?;
        let account = spec.keyring_account.clone();
        let attributes =
            HashMap::from([("service", "ttsandsttp"), ("account", account.as_str())]);
        let search = ss
            .search_items(attributes)
            .await
            .context("Failed to search the keyring")?;

        let items: Vec<_> = search.unlocked.into_iter().chain(search.locked).collect();
        if items.is_empty() {
            tracing::info!("No keyring entry found for {}", account);
            return Ok(());
        }
        for item in items {
            item.delete().await.context("Failed to delete keyring entry")?;
        }
        Ok(())
    }

    /// Redact a secret for safe inclusion in logs/errors.
    pub fn redact(key: &str) -> String {
        if key.len() <= 8 {
            "****".to_string()
        } else {
            format!("{}…{}", &key[..4], &key[key.len() - 4..])
        }
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn from_credential(spec: &KeySpec) -> Result<Option<SecretString>> {
    let dir = match std::env::var_os("CREDENTIALS_DIRECTORY") {
        Some(dir) => PathBuf::from(dir),
        None => return Ok(None),
    };

    for name in &spec.credentials {
        let path = dir.join(name);
        if path.exists() {
            return Ok(Some(read_secret_file(&path, false)?));
        }
    }
    Ok(None)
}

fn from_file(spec: &KeySpec) -> Result<Option<SecretString>> {
    if !spec.key_file.exists() {
        return Ok(None);
    }
    Ok(Some(read_secret_file(&spec.key_file, true)?))
}

fn from_env(spec: &KeySpec) -> Option<SecretString> {
    std::env::var(&spec.env)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(SecretString::from)
}

fn read_secret_file(path: &std::path::Path, enforce_perms: bool) -> Result<SecretString> {
    #[cfg(unix)]
    if enforce_perms {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .with_context(|| format!("Failed to stat {}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            bail!(
                "Refusing to read {}: permissions {:o} are too open (chmod 600)",
                path.display(),
                mode & 0o777
            );
        }
    }

    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        bail!("API key file {} is empty", path.display());
    }
    Ok(SecretString::from(trimmed.to_string()))
}

async fn connect_keyring() -> Result<SecretService<'static>> {
    // Bound the connect so a missing/locked keyring doesn't stall startup.
    let connect = tokio::time::timeout(
        Duration::from_secs(5),
        SecretService::connect(EncryptionType::Dh),
    )
    .await
    .context("Timed out connecting to the Secret Service")?;

    connect.context("Failed to connect to the Secret Service (is a keyring running?)")
}

async fn from_keyring(spec: &KeySpec) -> Result<Option<SecretString>> {
    let ss = connect_keyring().await?;
    let account = spec.keyring_account.clone();
    let attributes = HashMap::from([("service", "ttsandsttp"), ("account", account.as_str())]);
    let search = ss
        .search_items(attributes)
        .await
        .context("Failed to search the keyring")?;

    let item = if let Some(item) = search.unlocked.into_iter().next() {
        item
    } else if let Some(item) = search.locked.into_iter().next() {
        item.unlock().await.context("Failed to unlock keyring entry")?;
        item
    } else {
        return Ok(None);
    };

    let secret = item.get_secret().await.context("Failed to read keyring secret")?;
    let value = String::from_utf8(secret)
        .context("Keyring entry is not valid UTF-8")?
        .trim()
        .to_string();
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(SecretString::from(value)))
}

/// Convenience: expose the secret as `&str` (used only at request time).
pub fn expose(secret: &SecretString) -> &str {
    secret.expose_secret()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ApiRegion;

    #[test]
    fn dashscope_spec_keeps_legacy_names() {
        let api = ApiConfig {
            region: ApiRegion::Beijing,
            ..ApiConfig::default()
        };
        let spec = KeySpec::dashscope(&api);
        assert_eq!(
            spec.credentials,
            vec!["dashscope-beijing".to_string(), "dashscope".to_string()]
        );
        assert_eq!(spec.keyring_account, "dashscope/beijing");
        assert!(spec.key_file.ends_with("dashscope.beijing.key"));
        assert_eq!(spec.env, "DASHSCOPE_API_KEY");
    }

    #[test]
    fn provider_spec_defaults() {
        let spec = KeySpec::provider("groq", "Groq", "GROQ_API_KEY", "", "", KeySource::Auto);
        assert_eq!(spec.credentials, vec!["groq".to_string()]);
        assert_eq!(spec.keyring_account, "groq");
        assert!(spec.key_file.ends_with("groq.key"));
        assert_eq!(spec.env, "GROQ_API_KEY");
    }

    #[test]
    fn provider_spec_env_fallback_and_override() {
        let spec = KeySpec::provider("vllm-lan", "", "", "", "", KeySource::Auto);
        assert_eq!(spec.label, "vllm-lan");
        assert_eq!(spec.env, "VLLM_LAN_API_KEY");

        let spec = KeySpec::provider("vllm", "vLLM", "X", "MY_KEY", "~/k.txt", KeySource::File);
        assert_eq!(spec.env, "MY_KEY");
        assert!(spec.key_file.ends_with("k.txt"));
    }
}
