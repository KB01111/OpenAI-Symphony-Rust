//! Runtime detection of installed coding-agent CLIs.
//!
//! Detection is intentionally conservative: it locates the provider binary on `PATH`, asks it for
//! `--version` under a short timeout, and infers authentication only from the *presence* of
//! credential environment variables or credential files. Secret values are never read or returned.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::{json, Value};
use symphony_core::domain::{ProviderAuthState, ProviderKind, ProviderStatus};
use symphony_core::redaction::redact_secrets;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdout, Command};

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(4);
/// Upper bound for asking a CLI which models it offers (Codex starts its app-server for this).
const MODEL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Model catalogues change with CLI releases and accounts, not by the minute: provider detection runs
/// every minute while the UI is open, so a discovered list is reused for this long unless a refresh is
/// forced, rather than starting another process each time.
const MODEL_CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_DISCOVERED_MODELS: usize = 300;
const MAX_CODEX_MODEL_PAGES: u32 = 5;

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

/// Models a provider CLI reported for the current user.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveredModels {
    /// Models to offer, in the CLI's order (its default first when it marks one).
    pub models: Vec<String>,
    /// Every model id the CLI accepts, including ones hidden from its picker.
    pub known: HashSet<String>,
    /// The model the CLI uses when none is given.
    pub default: Option<String>,
}

/// A model id as a CLI prints it: no whitespace or control characters, bounded length.
fn plausible_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '@'))
}

/// One page of a Codex `model/list` response: visible ids, all ids, the default and the next cursor.
pub fn parse_codex_model_page(
    result: &Value,
) -> (Vec<String>, Vec<String>, Option<String>, Option<String>) {
    let mut visible = Vec::new();
    let mut all = Vec::new();
    let mut default = None;
    for entry in result
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = entry
            .get("id")
            .or_else(|| entry.get("model"))
            .and_then(Value::as_str)
            .filter(|id| plausible_model_id(id))
        else {
            continue;
        };
        all.push(id.to_string());
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if entry.get("isDefault").and_then(Value::as_bool) == Some(true) && default.is_none() {
            default = Some(id.to_string());
        }
        visible.push(id.to_string());
    }
    let next = result
        .get("nextCursor")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    (visible, all, default, next)
}

/// `opencode models` prints one `provider/model` per line.
pub fn parse_opencode_models(output: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    output
        .lines()
        .map(str::trim)
        .filter(|line| line.contains('/') && plausible_model_id(line))
        .filter(|line| seen.insert(line.to_string()))
        .take(MAX_DISCOVERED_MODELS)
        .map(str::to_string)
        .collect()
}

fn probe_command(binary: &Path) -> Command {
    let mut cmd = Command::new(binary);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(std::env::temp_dir())
        .kill_on_drop(true);
    symphony_workspaces::hide_console_window(&mut cmd);
    for key in symphony_workspaces::SECRET_ENV_VARS_TO_STRIP {
        cmd.env_remove(key);
    }
    cmd
}

async fn read_response(lines: &mut Lines<BufReader<ChildStdout>>, id: u64) -> Option<Value> {
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if message.get("id").and_then(Value::as_u64) == Some(id) && message.get("method").is_none()
        {
            return message.get("result").cloned();
        }
    }
    None
}

/// Ask `codex app-server` for its model catalogue (`model/list`). No turn is started, so this never
/// makes a model call; the catalogue reflects the user's Codex login.
async fn discover_codex_models(binary: &Path) -> Option<DiscoveredModels> {
    let mut cmd = probe_command(binary);
    cmd.arg("app-server").stdin(Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    let mut lines = BufReader::new(child.stdout.take()?).lines();
    let exchange = async {
        let send = |message: Value| format!("{message}\n");
        let init = send(json!({
            "method": "initialize",
            "id": 1,
            "params": { "clientInfo": {
                "name": "symphony-desktop-detection",
                "version": env!("CARGO_PKG_VERSION")
            } }
        }));
        stdin.write_all(init.as_bytes()).await.ok()?;
        read_response(&mut lines, 1).await?;
        stdin
            .write_all(send(json!({ "method": "initialized" })).as_bytes())
            .await
            .ok()?;
        let mut found = DiscoveredModels::default();
        let mut cursor: Option<String> = None;
        for page in 0..MAX_CODEX_MODEL_PAGES {
            let id = 2 + u64::from(page);
            let request = send(json!({
                "method": "model/list",
                "id": id,
                "params": { "cursor": cursor, "includeHidden": true }
            }));
            stdin.write_all(request.as_bytes()).await.ok()?;
            let result = read_response(&mut lines, id).await?;
            let (visible, all, default, next) = parse_codex_model_page(&result);
            found.models.extend(visible);
            found.known.extend(all);
            found.default = found.default.or(default);
            match next {
                Some(next) if found.known.len() < MAX_DISCOVERED_MODELS => cursor = Some(next),
                _ => break,
            }
        }
        Some(found)
    };
    let found = tokio::time::timeout(MODEL_PROBE_TIMEOUT, exchange)
        .await
        .ok()
        .flatten();
    // Closing stdin lets the app-server exit on its own; the kill covers one that does not.
    drop(stdin);
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        let _ = child.start_kill();
    }
    let mut found = found?;
    found.models.truncate(MAX_DISCOVERED_MODELS);
    if let Some(default) = &found.default {
        if let Some(pos) = found.models.iter().position(|m| m == default) {
            let model = found.models.remove(pos);
            found.models.insert(0, model);
        }
    }
    (!found.models.is_empty()).then_some(found)
}

/// `opencode models`: the models of every provider the user configured in OpenCode.
async fn discover_opencode_models(binary: &Path) -> Option<DiscoveredModels> {
    let mut cmd = probe_command(binary);
    cmd.arg("models");
    let output = tokio::time::timeout(MODEL_PROBE_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let models = parse_opencode_models(&String::from_utf8_lossy(&output.stdout));
    (!models.is_empty()).then(|| DiscoveredModels {
        known: models.iter().cloned().collect(),
        models,
        default: None,
    })
}

type ModelCacheKey = (ProviderKind, PathBuf, Option<String>);

fn model_cache() -> &'static Mutex<HashMap<ModelCacheKey, (Instant, Option<DiscoveredModels>)>> {
    static CACHE: OnceLock<Mutex<HashMap<ModelCacheKey, (Instant, Option<DiscoveredModels>)>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Models the installed CLI offers, cached per binary and version (a failed probe is cached too,
/// so an older CLI without a model listing is not started again every minute).
async fn discover_models(
    kind: ProviderKind,
    binary: &Path,
    version: Option<&str>,
    refresh: bool,
) -> Option<DiscoveredModels> {
    let key = (kind, binary.to_path_buf(), version.map(str::to_string));
    if !refresh {
        if let Ok(cache) = model_cache().lock() {
            if let Some((at, found)) = cache.get(&key) {
                if at.elapsed() < MODEL_CACHE_TTL {
                    return found.clone();
                }
            }
        }
    }
    let found = match kind {
        ProviderKind::Codex => discover_codex_models(binary).await,
        ProviderKind::OpenCode => discover_opencode_models(binary).await,
        // Claude Code has no model listing; its aliases (`opus`, `sonnet`, …) never go stale.
        ProviderKind::ClaudeCode | ProviderKind::Simulated => None,
    };
    if let Ok(mut cache) = model_cache().lock() {
        cache.insert(key, (Instant::now(), found.clone()));
    }
    found
}

/// Apply a discovered catalogue: it replaces the built-in list, and a configured default the CLI does
/// not accept is not used (it would fail every run); the status message says so. `None` means "no
/// `--model`": the CLI's own default.
fn apply_discovered_models(
    capabilities: &mut symphony_core::domain::ProviderCapabilities,
    configured_default: Option<String>,
    discovered: Option<DiscoveredModels>,
) -> Option<String> {
    let Some(discovered) = discovered else {
        return configured_default.or_else(|| capabilities.models.first().cloned());
    };
    capabilities.models = discovered.models.clone();
    // Only a default the CLI names itself: OpenCode lists models without one, and then its own
    // configuration decides (no `--model`), not whichever model happens to be listed first.
    let fallback = discovered.default.clone();
    match configured_default {
        Some(model) if discovered.known.contains(&model) => Some(model),
        Some(model) => {
            capabilities.status_message = format!(
                "{} · Configured default model `{model}` is not offered by this CLI; using {}.",
                capabilities.status_message,
                fallback
                    .as_deref()
                    .map(|m| format!("`{m}`"))
                    .unwrap_or_else(|| "the CLI's default".to_string())
            );
            fallback
        }
        None => fallback,
    }
}

/// Detect a single provider's installation, version, authentication, and capabilities.
pub async fn detect_provider_status(
    kind: ProviderKind,
    vault_keys: &[String],
    default_model: Option<String>,
    refresh_models: bool,
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
    let discovered = match (&binary_path, provider_binary_name(kind)) {
        (Some(path), Some(_)) => {
            discover_models(kind, Path::new(path), version.as_deref(), refresh_models).await
        }
        _ => None,
    };
    let default_model = apply_discovered_models(&mut capabilities, default_model, discovered);

    ProviderStatus {
        provider: kind,
        display_name: kind.display_name().to_string(),
        installed,
        binary_path,
        version,
        auth_state,
        auth_detail,
        default_model,
        capabilities,
        detected_at: Utc::now(),
    }
}

/// Detect every supported provider concurrently.
pub async fn detect_provider_statuses(
    vault_keys: &[String],
    default_model_for: impl Fn(ProviderKind) -> Option<String>,
    refresh_models: bool,
) -> Vec<ProviderStatus> {
    let kinds = [
        ProviderKind::Codex,
        ProviderKind::ClaudeCode,
        ProviderKind::OpenCode,
        ProviderKind::Simulated,
    ];
    let futures = kinds.iter().map(|kind| {
        detect_provider_status(*kind, vault_keys, default_model_for(*kind), refresh_models)
    });
    futures::future::join_all(futures).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simulated_provider_is_always_installed_without_auth() {
        let status = detect_provider_status(ProviderKind::Simulated, &[], None, false).await;
        assert!(status.installed);
        assert_eq!(status.auth_state, ProviderAuthState::NotRequired);
        assert!(status.capabilities.available);
        assert_eq!(
            status.default_model.as_deref(),
            Some("simulated-deterministic-v1")
        );
    }

    #[test]
    fn codex_model_page_keeps_hidden_ids_known_but_unlisted() {
        let page = json!({
            "data": [
                { "id": "gpt-a", "hidden": false, "isDefault": false },
                { "id": "gpt-old", "hidden": true, "isDefault": false },
                { "id": "gpt-b", "isDefault": true },
                { "id": "has space" },
                { "model": "gpt-c" }
            ],
            "nextCursor": "page-2"
        });
        let (visible, all, default, next) = parse_codex_model_page(&page);
        assert_eq!(visible, ["gpt-a", "gpt-b", "gpt-c"]);
        assert_eq!(all, ["gpt-a", "gpt-old", "gpt-b", "gpt-c"]);
        assert_eq!(default.as_deref(), Some("gpt-b"));
        assert_eq!(next.as_deref(), Some("page-2"));
        let (_, _, _, last) = parse_codex_model_page(&json!({ "data": [], "nextCursor": null }));
        assert_eq!(last, None);
    }

    #[test]
    fn opencode_models_are_provider_qualified_lines() {
        let out = "opencode/big-pickle\n\n anthropic/claude-opus-5-5 \nWARN something went wrong\namazon-bedrock/anthropic.claude-sonnet-5-5\nanthropic/claude-opus-5-5\nplain\n";
        assert_eq!(
            parse_opencode_models(out),
            [
                "opencode/big-pickle",
                "anthropic/claude-opus-5-5",
                "amazon-bedrock/anthropic.claude-sonnet-5-5"
            ]
        );
    }

    fn discovered(models: &[&str], hidden: &[&str], default: Option<&str>) -> DiscoveredModels {
        DiscoveredModels {
            models: models.iter().map(|m| m.to_string()).collect(),
            known: models.iter().chain(hidden).map(|m| m.to_string()).collect(),
            default: default.map(str::to_string),
        }
    }

    #[test]
    fn a_configured_default_the_cli_does_not_accept_falls_back_visibly() {
        let mut caps = ProviderKind::Codex.default_capabilities();
        let found = discovered(&["gpt-new", "gpt-mini"], &["gpt-legacy"], Some("gpt-new"));

        // Without discovery the configured default (or the built-in list) stands.
        let mut plain = caps.clone();
        assert_eq!(
            apply_discovered_models(&mut plain, Some("gpt-5-codex".into()), None).as_deref(),
            Some("gpt-5-codex")
        );
        assert_eq!(plain, caps);

        // A hidden but accepted model stays the user's choice.
        let kept =
            apply_discovered_models(&mut caps, Some("gpt-legacy".into()), Some(found.clone()));
        assert_eq!(kept.as_deref(), Some("gpt-legacy"));
        assert_eq!(caps.models, ["gpt-new", "gpt-mini"]);

        // A retired one is replaced by the CLI's default, and the status says why.
        let mut caps = ProviderKind::Codex.default_capabilities();
        let replaced =
            apply_discovered_models(&mut caps, Some("gpt-5-codex".into()), Some(found.clone()));
        assert_eq!(replaced.as_deref(), Some("gpt-new"));
        assert!(caps.status_message.contains("`gpt-5-codex` is not offered"));

        // No configured default: the CLI's own.
        let mut caps = ProviderKind::Codex.default_capabilities();
        assert_eq!(
            apply_discovered_models(&mut caps, None, Some(found)).as_deref(),
            Some("gpt-new")
        );

        // A CLI that names no default (OpenCode) keeps its own configuration: no model is forced.
        let mut caps = ProviderKind::OpenCode.default_capabilities();
        let listed = discovered(&["x/a", "x/b"], &[], None);
        assert_eq!(
            apply_discovered_models(&mut caps, None, Some(listed.clone())),
            None
        );
        let replaced = apply_discovered_models(&mut caps, Some("y/gone".into()), Some(listed));
        assert_eq!(replaced, None);
        assert!(caps.status_message.contains("using the CLI's default"));
    }

    #[cfg(unix)]
    fn fake_cli(dir: &Path, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// The exchange with a real process: initialize, initialized, then `model/list` pages until the
    /// cursor ends, skipping notifications; the app-server exits when stdin closes.
    #[cfg(unix)]
    #[tokio::test]
    async fn codex_catalogue_is_read_from_the_app_server_across_pages() {
        let dir = tempfile::tempdir().unwrap();
        let codex = fake_cli(
            dir.path(),
            "codex",
            r#"[ "$1" = "app-server" ] || exit 2
while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*) echo 'not json'; echo '{"id":1,"result":{}}' ;;
    *'"model/list"'*'"cursor":null'*)
      echo '{"method":"configWarning","params":{}}'
      echo '{"id":2,"result":{"data":[{"id":"m-a"},{"id":"m-hidden","hidden":true},{"id":"m-b","isDefault":true}],"nextCursor":"p2"}}' ;;
    *'"model/list"'*'"cursor":"p2"'*) echo '{"id":3,"result":{"data":[{"id":"m-c"}],"nextCursor":null}}' ;;
  esac
done
"#,
        );
        let found = discover_codex_models(&codex).await.expect("catalogue");
        assert_eq!(found.models, ["m-b", "m-a", "m-c"]);
        assert_eq!(found.default.as_deref(), Some("m-b"));
        assert!(found.known.contains("m-hidden"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_cli_without_a_model_listing_reports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let codex = fake_cli(
            dir.path(),
            "codex",
            "echo '{\"id\":1,\"result\":{}}'\nexit 0\n",
        );
        assert_eq!(discover_codex_models(&codex).await, None);
        let opencode = fake_cli(
            dir.path(),
            "opencode",
            "echo 'Error: unknown command' >&2\nexit 1\n",
        );
        assert_eq!(discover_opencode_models(&opencode).await, None);
        let opencode = fake_cli(
            dir.path(),
            "opencode2",
            "[ \"$1\" = models ] && printf 'x/y\\nx/z\\n'\n",
        );
        assert_eq!(
            discover_opencode_models(&opencode).await.map(|d| d.models),
            Some(vec!["x/y".to_string(), "x/z".to_string()])
        );
    }

    /// Opt-in check against a real CLI (no login, no model call):
    /// `SYMPHONY_REAL_CODEX=/path/to/codex cargo test -p symphony-agents -- --ignored real_codex`.
    #[tokio::test]
    #[ignore = "needs a real codex binary"]
    async fn real_codex_reports_its_catalogue() {
        let codex =
            PathBuf::from(std::env::var("SYMPHONY_REAL_CODEX").expect("SYMPHONY_REAL_CODEX"));
        let found = discover_codex_models(&codex).await.expect("catalogue");
        assert!(found.default.is_some());
        assert_eq!(found.models.first(), found.default.as_ref());
        println!("{found:?}");
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
