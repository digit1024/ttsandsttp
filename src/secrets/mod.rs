//! API key resolution with a secure, layered fallback chain.
//!
//! Resolution order for [`KeySource::Auto`]:
//! 1. systemd `$CREDENTIALS_DIRECTORY` (`LoadCredential=`)
//! 2. Freedesktop Secret Service (GNOME Keyring / KWallet)
//! 3. A `0600` file in `~/.config/ttsandsttp/`
//! 4. The environment variable (development only)
//!
//! The resolved key is wrapped in [`SecretString`] (zeroized on drop) and is
//! never logged.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use secrecy::{ExposeSecret, SecretString};
use secret_service::{EncryptionType, SecretService};

use crate::config::{ApiConfig, KeySource};

/// Resolved secret storage helpers
pub struct Secrets;

impl Secrets {
    /// Resolve the API key according to the configured [`KeySource`].
    pub async fn resolve(api: &ApiConfig) -> Result<SecretString> {
        match api.key_source {
            KeySource::Auto => {
                if let Some(key) = from_credential(api)? {
                    tracing::debug!("API key resolved from systemd credentials");
                    return Ok(key);
                }
                match from_keyring(api).await {
                    Ok(Some(key)) => {
                        tracing::debug!("API key resolved from Secret Service keyring");
                        return Ok(key);
                    }
                    Ok(None) => {}
                    Err(e) => tracing::debug!("Secret Service lookup unavailable: {}", e),
                }
                if let Some(key) = from_file(api)? {
                    tracing::debug!("API key resolved from key file");
                    return Ok(key);
                }
                if let Some(key) = from_env(api) {
                    tracing::warn!(
                        "API key resolved from environment variable ({}); \
                         prefer `ttsandsttp set-key` for secure storage",
                        api.effective_env()
                    );
                    return Ok(key);
                }
                bail!(
                    "No DashScope API key found (region '{}'). Store one with \
                     `ttsandsttp set-key`, set it via systemd LoadCredential, or export {}",
                    api.region.slug(),
                    api.effective_env()
                )
            }
            KeySource::Credential => from_credential(api)?.context(
                "No API key in $CREDENTIALS_DIRECTORY (add LoadCredential=dashscope-<region>:<path>)",
            ),
            KeySource::Keyring => from_keyring(api)
                .await?
                .context("No API key in the Secret Service keyring (run `ttsandsttp set-key`)"),
            KeySource::File => from_file(api)?.context(format!(
                "No API key file at {}",
                key_file_path(api).display()
            )),
            KeySource::Env => from_env(api).context(format!(
                "Environment variable {} is not set",
                api.effective_env()
            )),
        }
    }

    /// Store the API key in the Secret Service keyring, replacing any existing entry.
    pub async fn store(api: &ApiConfig, key: &str) -> Result<()> {
        let key = key.trim();
        if key.is_empty() {
            bail!("Refusing to store an empty API key");
        }

        let account = keyring_account(api);
        let ss = connect_keyring().await?;
        let collection = ss
            .get_default_collection()
            .await
            .context("Failed to open the default keyring collection")?;
        collection
            .ensure_unlocked()
            .await
            .context("Failed to unlock the keyring")?;

        let attributes = HashMap::from([("service", "ttsandsttp"), ("account", account.as_str())]);
        collection
            .create_item(
                &format!("ttsandsttp DashScope API key ({})", api.region.slug()),
                attributes,
                key.as_bytes(),
                true,
                "text/plain",
            )
            .await
            .context("Failed to store the key in the keyring")?;

        Ok(())
    }

    /// Remove the API key from the Secret Service keyring.
    pub async fn delete(api: &ApiConfig) -> Result<()> {
        let ss = connect_keyring().await?;
        let account = keyring_account(api);
        let attributes = HashMap::from([("service", "ttsandsttp"), ("account", account.as_str())]);
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

fn keyring_account(api: &ApiConfig) -> String {
    format!("dashscope/{}", api.region.slug())
}

fn key_file_path(api: &ApiConfig) -> PathBuf {
    if !api.key_file.is_empty() {
        return expand_tilde(&api.key_file);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("ttsandsttp")
        .join(format!("dashscope.{}.key", api.region.slug()))
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn from_credential(api: &ApiConfig) -> Result<Option<SecretString>> {
    let dir = match std::env::var_os("CREDENTIALS_DIRECTORY") {
        Some(dir) => PathBuf::from(dir),
        None => return Ok(None),
    };

    // Try region-scoped name first, then a generic name.
    for name in [
        format!("dashscope-{}", api.region.slug()),
        "dashscope".to_string(),
    ] {
        let path = dir.join(&name);
        if path.exists() {
            return Ok(Some(read_secret_file(&path, false)?));
        }
    }
    Ok(None)
}

fn from_file(api: &ApiConfig) -> Result<Option<SecretString>> {
    let path = key_file_path(api);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_secret_file(&path, true)?))
}

fn from_env(api: &ApiConfig) -> Option<SecretString> {
    let var = api.effective_env();
    std::env::var(&var)
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

async fn from_keyring(api: &ApiConfig) -> Result<Option<SecretString>> {
    let ss = connect_keyring().await?;
    let account = keyring_account(api);
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
