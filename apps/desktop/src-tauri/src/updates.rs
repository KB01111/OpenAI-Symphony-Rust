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
use std::time::{Duration, Instant};

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
    app.updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())
}

/// Download the staged update, verify its signature, and run the installer.
///
/// `running_tasks` is the number of agent runs the embedded runtime has in flight (`None` when the
/// runtime is owned by another process). Installing ends the app, and with it those runs; they are
/// re-queued by crash recovery, so the user has to confirm first.
pub async fn install(
    app: &AppHandle,
    pending: &PendingUpdate,
    running_tasks: Option<usize>,
    confirmed: bool,
) -> Result<InstallOutcome, String> {
    if let Some(running) = running_tasks.filter(|n| *n > 0 && !confirmed) {
        return Ok(InstallOutcome::NeedsConfirmation { running });
    }
    if pending.installing.swap(true, Ordering::AcqRel) {
        return Err("An update is already being installed".to_string());
    }
    let result = download_and_run(app, pending).await;
    pending.installing.store(false, Ordering::Release);
    result
}

async fn download_and_run(
    app: &AppHandle,
    pending: &PendingUpdate,
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
}
