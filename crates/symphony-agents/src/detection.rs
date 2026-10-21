//! Runtime detection of installed coding-agent CLIs.
//!
//! Detection is intentionally conservative: it locates the provider binary on `PATH`, asks it for
//! `--version` under a short timeout, and infers authentication only from the *presence* of
//! credential environment variables or credential files. Secret values are never read or returned.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::Utc;
use symphony_core::domain::{ProviderAuthState, ProviderKind, ProviderStatus};
use symphony_core::redaction::redact_secrets;
use tokio::process::Command;

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(4);

/// Binary name looked up on `PATH` for each external provider.
pub fn provider_binary_name(kind: ProviderKind) -> Option<&'static str> {
    match kind {
        ProviderKind::Codex => Some("codex"),
        ProviderKind::ClaudeCode => Some("claude"),
        ProviderKind::OpenCode => Some("opencode"),
        ProviderKind::Simulated => None,
    }
}

/// Locate an executable on `PATH` (honouring `PATHEXT` on Windows).
pub fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path_var) {
        for ext in &extensions {
            let candidate = dir.join(format!("{binary}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

async fn probe_version(binary: &Path) -> Option<String> {
    let mut cmd = Command::new(binary);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    symphony_workspaces::hide_console_window(&mut cmd);
    for key in symphony_workspaces::SECRET_ENV_VARS_TO_STRIP {
        cmd.env_remove(key);
    }
    let output = tokio::time::timeout(VERSION_PROBE_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(redact_secrets(
        &line.chars().take(120).collect::<String>(),
        &[],
    ))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn env_present(key: &str) -> bool {
    std::env::var(key)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// Infer authentication from credential sources. `vault_keys` are the key *names* configured in the
/// Symphony credential vault.
pub fn detect_auth(
    kind: ProviderKind,
    vault_keys: &[String],
) -> (ProviderAuthState, Option<String>) {
    let in_vault = |key: &str| vault_keys.iter().any(|k| k == key);
    let home = home_dir();
    match kind {
        ProviderKind::Simulated => (ProviderAuthState::NotRequired, None),
        ProviderKind::Codex => {
            if env_present("OPENAI_API_KEY") {
                return (
                    ProviderAuthState::Authenticated,
                    Some("OPENAI_API_KEY environment variable".into()),
                );
            }
            if in_vault("OPENAI_API_KEY") {
                return (
                    ProviderAuthState::Authenticated,
                    Some("OPENAI_API_KEY in Symphony credential vault".into()),
                );
            }
            let codex_home = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(".codex")));
            if codex_home
                .map(|dir| dir.join("auth.json").is_file())
                .unwrap_or(false)
            {
                return (
                    ProviderAuthState::Authenticated,
                    Some("Codex CLI login (auth.json)".into()),
                );
            }
            (
                ProviderAuthState::Unauthenticated,
                Some("Run `codex login` or configure OPENAI_API_KEY".into()),
            )
        }
        ProviderKind::ClaudeCode => {
            if env_present("ANTHROPIC_API_KEY") {
                return (
                    ProviderAuthState::Authenticated,
                    Some("ANTHROPIC_API_KEY environment variable".into()),
                );
            }
            if in_vault("ANTHROPIC_API_KEY") {
                return (
                    ProviderAuthState::Authenticated,
                    Some("ANTHROPIC_API_KEY in Symphony credential vault".into()),
                );
            }
            if home
                .map(|h| h.join(".claude").join(".credentials.json").is_file())
                .unwrap_or(false)
            {
                return (
                    ProviderAuthState::Authenticated,
                    Some("Claude Code login (.credentials.json)".into()),
                );
            }
            (
                ProviderAuthState::Unknown,
                Some(
                    "Credentials may be stored in the OS keychain; run `claude` to sign in".into(),
                ),
            )
        }
        ProviderKind::OpenCode => {
            let data_home = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| home.map(|h| h.join(".local").join("share")));
            if data_home
                .map(|d| d.join("opencode").join("auth.json").is_file())
                .unwrap_or(false)
            {
                return (
                    ProviderAuthState::Authenticated,
                    Some("OpenCode login (auth.json)".into()),
                );
            }
            (
                ProviderAuthState::Unknown,
                Some("Run `opencode auth login` to configure a provider".into()),
            )
        }
    }
}

/// Detect a single provider's installation, version, authentication, and capabilities.
pub async fn detect_provider_status(
    kind: ProviderKind,
    vault_keys: &[String],
    default_model: Option<String>,
) -> ProviderStatus {
    let mut capabilities = kind.default_capabilities();
    let (installed, binary_path, version) = match provider_binary_name(kind) {
        None => (
            true,
            capabilities.binary_path.clone(),
            capabilities.version.clone(),
        ),
        Some(binary) => match find_on_path(binary) {
            Some(path) => {
                let version = probe_version(&path).await;
                (true, Some(path.to_string_lossy().to_string()), version)
            }
            None => (false, None, None),
        },
    };
    capabilities.available = installed;
    capabilities.binary_path = binary_path.clone();
    capabilities.version = version.clone();
    let (auth_state, auth_detail) = detect_auth(kind, vault_keys);

    ProviderStatus {
        provider: kind,
        display_name: kind.display_name().to_string(),
        installed,
        binary_path,
        version,
        auth_state,
        auth_detail,
        default_model: default_model.or_else(|| capabilities.models.first().cloned()),
        capabilities,
        detected_at: Utc::now(),
    }
}

/// Detect every supported provider concurrently.
pub async fn detect_provider_statuses(
    vault_keys: &[String],
    default_model_for: impl Fn(ProviderKind) -> Option<String>,
) -> Vec<ProviderStatus> {
    let kinds = [
        ProviderKind::Codex,
        ProviderKind::ClaudeCode,
        ProviderKind::OpenCode,
        ProviderKind::Simulated,
    ];
    let futures = kinds
        .iter()
        .map(|kind| detect_provider_status(*kind, vault_keys, default_model_for(*kind)));
    futures::future::join_all(futures).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simulated_provider_is_always_installed_without_auth() {
        let status = detect_provider_status(ProviderKind::Simulated, &[], None).await;
        assert!(status.installed);
        assert_eq!(status.auth_state, ProviderAuthState::NotRequired);
        assert!(status.capabilities.available);
        assert_eq!(
            status.default_model.as_deref(),
            Some("simulated-deterministic-v1")
        );
    }

    #[test]
    fn missing_binary_is_not_found() {
        assert!(find_on_path("symphony-definitely-not-a-real-binary").is_none());
    }

    #[test]
    fn vault_key_counts_as_authenticated_without_reading_value() {
        let (state, detail) = detect_auth(ProviderKind::Codex, &["OPENAI_API_KEY".to_string()]);
        if !env_present("OPENAI_API_KEY") {
            assert_eq!(state, ProviderAuthState::Authenticated);
            assert!(detail.unwrap().contains("vault"));
        }
    }
}
