//! In-app updates from GitHub releases.
//!
//! Two steps, both started by the webview but neither taking a URL or a path from it:
//!
//! 1. [`check`] asks the GitHub releases API for the newest release that is relevant to the running
//!    build (pre-releases only for pre-release builds). That decides the *channel*.
//! 2. When that release carries a `latest.json` manifest, the Tauri updater reads the manifest
//!    (`releases/download/<tag>/latest.json`), and [`install`] downloads the installer it names,
//!    verifies its minisign signature against the public key compiled into the app, and only then
//!    runs it. A release without a manifest (e.g. one built before the update key was configured)
//!    is still announced, and the user is sent to the releases page instead.
//!
//! The endpoint hosts are compile-time constants. See `docs/RELEASING.md` for the signing keys.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use symphony_orchestrator::SymphonyOrchestrator;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

use semver::Version;
use serde::{Deserialize, Serialize};

/// Where releases are published; the only URL the host opens for "download the update".
pub const RELEASES_URL: &str = "https://github.com/KB01111/OpenAI-Symphony-Rust/releases";
const RELEASES_API: &str =
    "https://api.github.com/repos/KB01111/OpenAI-Symphony-Rust/releases?per_page=30";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);
const MAX_NOTES_CHARS: usize = 4000;
/// The release asset that makes a release installable from inside the app.
pub const MANIFEST_ASSET: &str = "latest.json";
const RELEASE_DOWNLOADS: &str = "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download";
/// Event the webview listens to for download progress.
pub const PROGRESS_EVENT: &str = "symphony://update-progress";

#[derive(Debug, Deserialize)]
struct ReleaseDto {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
    body: Option<String>,
    #[serde(default)]
    assets: Vec<AssetDto>,
}

#[derive(Debug, Deserialize)]
struct AssetDto {
    name: String,
}

/// Result handed to the webview.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct UpdateCheck {
    pub current: String,
    /// Newest published version, `None` when no usable release exists yet.
    pub latest: Option<String>,
    pub update_available: bool,
    pub prerelease: bool,
    pub published_at: Option<String>,
    /// Release notes (truncated, plain text from the release body).
    pub notes: Option<String>,
    /// Whether the app can download and install this update itself: the release has a signed
    /// manifest and the updater accepted it. When `false` the user installs from the releases page.
    pub installable: bool,
    /// Manifest of the newest release; never sent to the webview.
    #[serde(skip)]
    manifest_url: Option<String>,
}

fn manifest_url_for(release: &ReleaseDto) -> Option<String> {
    release
        .assets
        .iter()
        .any(|a| a.name == MANIFEST_ASSET)
        .then(|| format!("{RELEASE_DOWNLOADS}/{}/{MANIFEST_ASSET}", release.tag_name))
}

fn parse_tag(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

/// Pick the newest release that is relevant to `current`: drafts never count, and releases GitHub
/// marks as pre-releases only count when the running build is itself a pre-release.
fn evaluate(current: &str, releases: Vec<ReleaseDto>) -> Result<UpdateCheck, String> {
    let running = Version::parse(current).map_err(|e| format!("invalid running version: {e}"))?;
    let allow_pre = !running.pre.is_empty();
    let newest = releases
        .into_iter()
        .filter(|r| !r.draft && (allow_pre || !r.prerelease))
        .filter_map(|r| parse_tag(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, _)| allow_pre || v.pre.is_empty())
        .max_by(|(a, _), (b, _)| a.cmp(b));
    Ok(match newest {
        Some((version, release)) => UpdateCheck {
            installable: false,
            manifest_url: manifest_url_for(&release),
            current: current.to_string(),
            update_available: version > running,
            latest: Some(version.to_string()),
            prerelease: release.prerelease || !version.pre.is_empty(),
            published_at: release.published_at,
            notes: release
                .body
                .map(|b| b.trim().chars().take(MAX_NOTES_CHARS).collect::<String>())
                .filter(|b| !b.is_empty()),
        },
        None => UpdateCheck {
            installable: false,
            manifest_url: None,
            current: current.to_string(),
            latest: None,
            update_available: false,
            prerelease: false,
            published_at: None,
            notes: None,
        },
    })
}

/// Ask GitHub for the published releases and compare with `current`.
pub async fn check(current: &str) -> Result<UpdateCheck, String> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(format!("symphony-desktop/{current}"))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Could not reach GitHub: {}", e.without_url()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            403 | 429 => "GitHub rate-limited the request; try again later".to_string(),
            code => format!("GitHub answered HTTP {code}"),
        });
    }
    let releases: Vec<ReleaseDto> = response
        .json()
        .await
        .map_err(|e| format!("Unexpected answer from GitHub: {}", e.without_url()))?;
    evaluate(current, releases)
}

/// The update found by the last check, kept host-side so the webview never names what to install.
#[derive(Default)]
pub struct PendingUpdate {
    update: Mutex<Option<Update>>,
    installing: AtomicBool,
}

/// Download progress reported to the webview on [`PROGRESS_EVENT`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UpdateProgress {
    /// `downloading` | `installing`
    pub phase: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// What `install_update` did.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InstallOutcome {
    /// Tasks are running and the caller did not confirm; nothing was downloaded.
    NeedsConfirmation { running: usize },
    /// The installer ran and the app is closing (Windows) or restarting (elsewhere).
    Installed,
}

/// Look for a newer release and, when it can be installed in-app, remember it for [`install`].
pub async fn check_and_stage(
    app: &AppHandle,
    pending: &PendingUpdate,
) -> Result<UpdateCheck, String> {
    let mut check = check(env!("CARGO_PKG_VERSION")).await?;
    *pending.update.lock().await = None;
    if !check.update_available {
        return Ok(check);
    }
    let Some(manifest) = check.manifest_url.clone() else {
        return Ok(check);
    };
    match find_signed_update(app, &manifest).await {
        Ok(Some(update)) if Some(update.version.as_str()) == check.latest.as_deref() => {
            *pending.update.lock().await = Some(update);
            check.installable = true;
        }
        Ok(_) => tracing::warn!("The release manifest does not offer the announced version"),
        // A broken manifest must not hide the update: the user can still install it by hand.
        Err(e) => tracing::warn!("Could not read the update manifest: {e}"),
    }
    Ok(check)
}

async fn find_signed_update(app: &AppHandle, manifest: &str) -> Result<Option<Update>, String> {
    let url = manifest
        .parse()
        .map_err(|e| format!("bad manifest URL: {e}"))?;
    let update = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(update) = &update {
        if !download_is_from_release(manifest, update.download_url.as_str()) {
            return Err("the manifest points to an installer outside its own release".to_string());
        }
    }
    Ok(update)
}

/// The manifest is not signed, only the installer is, so a doctored manifest could pair a newer
/// version number with another genuinely signed installer (an older build). Requiring the
/// installer to live in the same release as the manifest keeps the source to this repository's
/// own release assets.
fn download_is_from_release(manifest_url: &str, download_url: &str) -> bool {
    manifest_url
        .strip_suffix(MANIFEST_ASSET)
        .is_some_and(|release| {
            release.ends_with('/')
                && download_url.starts_with(release)
                && !download_url[release.len()..].contains(['/', '?', '#'])
        })
}

/// Agent runs the embedded runtime has in flight; `None` when another process owns the runtime
/// (it keeps running when this app is replaced). Fails closed: an unreadable count is an error,
/// never "zero".
async fn running_agents(
    orchestrator: Option<&SymphonyOrchestrator>,
) -> Result<Option<usize>, String> {
    match orchestrator {
        None => Ok(None),
        Some(orchestrator) => orchestrator
            .state_snapshot()
            .await
            .map(|s| Some(s.counts.running))
            .map_err(|e| format!("Could not tell whether agents are running: {e}")),
    }
}

/// Download the staged update, verify its signature, and run the installer.
///
/// Installing ends the app, and with it the agent runs of the embedded runtime (`orchestrator`);
/// they are re-queued by crash recovery, so the user has to confirm first. The count is read
/// before the download and again right before the installer starts, because the scheduler keeps
/// dispatching meanwhile and a download can take a while.
pub async fn install(
    app: &AppHandle,
    pending: &PendingUpdate,
    orchestrator: Option<Arc<SymphonyOrchestrator>>,
    confirmed: bool,
) -> Result<InstallOutcome, String> {
    if !confirmed {
        if let Some(running) = running_agents(orchestrator.as_deref())
            .await?
            .filter(|n| *n > 0)
        {
            return Ok(InstallOutcome::NeedsConfirmation { running });
        }
    }
    if pending.installing.swap(true, Ordering::AcqRel) {
        return Err("An update is already being installed".to_string());
    }
    let result = download_and_run(app, pending, orchestrator.as_deref(), confirmed).await;
    pending.installing.store(false, Ordering::Release);
    result
}

async fn download_and_run(
    app: &AppHandle,
    pending: &PendingUpdate,
    orchestrator: Option<&SymphonyOrchestrator>,
    confirmed: bool,
) -> Result<InstallOutcome, String> {
    let update = pending
        .update
        .lock()
        .await
        .clone()
        .ok_or("No update has been checked yet; check for updates first")?;
    tracing::info!(from = %update.current_version, to = %update.version, "Downloading update");

    let mut downloaded = 0u64;
    let mut last_emit = Instant::now() - PROGRESS_INTERVAL;
    let progress_app = app.clone();
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                if last_emit.elapsed() >= PROGRESS_INTERVAL {
                    last_emit = Instant::now();
                    let _ = progress_app.emit(
                        PROGRESS_EVENT,
                        UpdateProgress {
                            phase: "downloading",
                            downloaded,
                            total,
                        },
                    );
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    let _ = app.emit(
        PROGRESS_EVENT,
        UpdateProgress {
            phase: "installing",
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
        },
    );
    if !confirmed {
        if let Some(running) = running_agents(orchestrator).await?.filter(|n| *n > 0) {
            // Work started during the download: ask again instead of interrupting it.
            return Ok(InstallOutcome::NeedsConfirmation { running });
        }
    }
    tracing::info!(version = %update.version, "Update verified; starting the installer");
    // Verifies the signature again, then runs the installer. On Windows this exits the process.
    update
        .install(&bytes)
        .map_err(|e| format!("Installation failed: {e}"))?;
    // Windows has already exited for the installer; elsewhere restart through the normal exit
    // path so the runtime shuts down and its discovery file is removed.
    app.request_restart();
    Ok(InstallOutcome::Installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, draft: bool, prerelease: bool) -> ReleaseDto {
        ReleaseDto {
            tag_name: tag.to_string(),
            draft,
            prerelease,
            published_at: Some("2026-10-22T10:00:00Z".to_string()),
            body: Some(format!("  notes for {tag}  ")),
            assets: Vec::new(),
        }
    }

    #[test]
    fn newer_stable_release_is_offered() {
        let check = evaluate(
            "2.0.0",
            vec![
                release("v2.0.0", false, false),
                release("v2.1.0", false, false),
            ],
        )
        .unwrap();
        assert!(check.update_available);
        assert_eq!(check.latest.as_deref(), Some("2.1.0"));
        assert_eq!(check.notes.as_deref(), Some("notes for v2.1.0"));
    }

    #[test]
    fn same_or_older_version_is_up_to_date() {
        let same = evaluate("2.1.0", vec![release("v2.1.0", false, false)]).unwrap();
        assert!(!same.update_available);
        assert_eq!(same.latest.as_deref(), Some("2.1.0"));
        let older = evaluate("2.1.0", vec![release("v2.0.0", false, false)]).unwrap();
        assert!(!older.update_available);
    }

    #[test]
    fn ordering_is_semantic_not_lexical() {
        let check = evaluate(
            "2.0.0",
            vec![
                release("v2.9.0", false, false),
                release("v2.10.0", false, false),
            ],
        )
        .unwrap();
        assert_eq!(check.latest.as_deref(), Some("2.10.0"));
    }

    #[test]
    fn drafts_and_unparsable_tags_are_ignored() {
        let check = evaluate(
            "2.0.0",
            vec![
                release("v9.0.0", true, false),
                release("nightly", false, false),
                release("v2.0.1", false, false),
            ],
        )
        .unwrap();
        assert_eq!(check.latest.as_deref(), Some("2.0.1"));
    }

    #[test]
    fn prereleases_are_offered_only_to_prerelease_builds() {
        let releases = || {
            vec![
                release("v2.0.0", false, false),
                release("v2.1.0-rc.1", false, true),
                // A stable-looking tag that GitHub flags as a pre-release (e.g. an unsigned build).
                release("v2.0.5", false, true),
            ]
        };
        let stable = evaluate("2.0.0", releases()).unwrap();
        assert!(
            !stable.update_available,
            "stable users never see pre-releases"
        );
        assert_eq!(stable.latest.as_deref(), Some("2.0.0"));

        let pre = evaluate("2.1.0-beta.1", releases()).unwrap();
        assert!(pre.update_available);
        assert_eq!(pre.latest.as_deref(), Some("2.1.0-rc.1"));
        assert!(pre.prerelease);
    }

    #[test]
    fn no_release_yet_is_not_an_error() {
        let check = evaluate("2.0.0", vec![]).unwrap();
        assert_eq!(check.latest, None);
        assert!(!check.update_available);
    }

    #[test]
    fn notes_are_truncated_and_empty_notes_dropped() {
        let mut long = release("v2.1.0", false, false);
        long.body = Some("x".repeat(MAX_NOTES_CHARS * 2));
        let check = evaluate("2.0.0", vec![long]).unwrap();
        assert_eq!(check.notes.unwrap().chars().count(), MAX_NOTES_CHARS);

        let mut blank = release("v2.1.0", false, false);
        blank.body = Some("   ".to_string());
        assert_eq!(evaluate("2.0.0", vec![blank]).unwrap().notes, None);
    }

    #[test]
    fn invalid_running_version_is_reported() {
        assert!(evaluate("not-a-version", vec![]).is_err());
    }

    #[test]
    fn github_payload_shape_deserializes() {
        let json = r#"[{"tag_name":"v2.1.0","draft":false,"prerelease":false,
            "published_at":"2026-10-22T10:00:00Z","body":null,"assets":[],"id":1}]"#;
        let releases: Vec<ReleaseDto> = serde_json::from_str(json).unwrap();
        assert_eq!(releases[0].tag_name, "v2.1.0");
        assert!(evaluate("2.0.0", releases).unwrap().update_available);
    }

    fn with_assets(tag: &str, assets: &[&str]) -> ReleaseDto {
        let mut r = release(tag, false, false);
        r.assets = assets
            .iter()
            .map(|n| AssetDto {
                name: n.to_string(),
            })
            .collect();
        r
    }

    #[test]
    fn manifest_url_exists_only_for_releases_that_ship_one() {
        let with = evaluate(
            "2.0.0",
            vec![with_assets("v2.1.0", &["setup.exe", "latest.json"])],
        )
        .unwrap();
        assert_eq!(
            with.manifest_url.as_deref(),
            Some("https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.0/latest.json")
        );
        let without = evaluate("2.0.0", vec![with_assets("v2.1.0", &["setup.exe"])]).unwrap();
        assert_eq!(without.manifest_url, None);
        assert!(
            !without.installable,
            "a check alone never marks a release installable"
        );
    }

    #[test]
    fn manifest_of_the_newest_release_is_used_not_an_older_one() {
        let check = evaluate(
            "2.0.0",
            vec![
                with_assets("v2.1.0", &["latest.json"]),
                with_assets("v2.2.0", &[]),
            ],
        )
        .unwrap();
        assert_eq!(check.latest.as_deref(), Some("2.2.0"));
        assert_eq!(check.manifest_url, None);
    }

    #[test]
    fn the_manifest_url_never_reaches_the_webview() {
        let check = evaluate("2.0.0", vec![with_assets("v2.1.0", &["latest.json"])]).unwrap();
        let json = serde_json::to_value(&check).unwrap();
        assert!(json.get("manifest_url").is_none());
        assert_eq!(json["installable"], false);
    }

    #[test]
    fn install_outcomes_are_tagged_for_the_webview() {
        let json = serde_json::to_value(InstallOutcome::NeedsConfirmation { running: 2 }).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"status": "needs_confirmation", "running": 2})
        );
        let json = serde_json::to_value(InstallOutcome::Installed).unwrap();
        assert_eq!(json, serde_json::json!({"status": "installed"}));
    }

    #[test]
    fn the_installer_must_come_from_the_manifests_own_release() {
        let manifest =
            "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.0/latest.json";
        let ok = "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.0/Symphony-Desktop_2.1.0_x64-setup.exe";
        assert!(download_is_from_release(manifest, ok));
        for bad in [
            // an older release of the same repository (a genuinely signed, older installer)
            "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v1.0.0/setup.exe",
            // another host or repository
            "https://evil.example/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.0/setup.exe",
            "https://github.com/other/OpenAI-Symphony-Rust/releases/download/v2.1.0/setup.exe",
            // sneaking out of the release directory
            "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.0/../v1.0.0/setup.exe",
            "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.0/x/setup.exe",
            "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.0/setup.exe?redirect=1",
            // a tag that merely starts like the release's
            "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.00/setup.exe",
        ] {
            assert!(!download_is_from_release(manifest, bad), "{bad}");
        }
        assert!(!download_is_from_release(
            "https://example.com/other.json",
            ok
        ));
    }

    /// The manifest is unsigned; this setting is what binds its version to the signed installer
    /// (docs/RELEASING.md), so it must not be dropped from the config by accident.
    #[test]
    fn the_updater_requires_the_signed_version() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            config["plugins"]["updater"]["requireSignedVersion"],
            serde_json::json!(true)
        );
    }
}
