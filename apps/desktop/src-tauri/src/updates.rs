//! "Check for updates": an explicit, user-initiated lookup of the newest GitHub release.
//!
//! This deliberately is not an auto-updater. It never downloads or runs anything, the endpoint is
//! a compile-time constant (no argument reaches the network layer), the app never checks on its
//! own, and installing stays a manual step from the releases page. Signed in-app updates need an
//! update-signing key and a hosted manifest (see `docs/RELEASING.md`).

use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};

/// Where releases are published; the only URL the host opens for "download the update".
pub const RELEASES_URL: &str = "https://github.com/KB01111/OpenAI-Symphony-Rust/releases";
const RELEASES_API: &str =
    "https://api.github.com/repos/KB01111/OpenAI-Symphony-Rust/releases?per_page=30";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_NOTES_CHARS: usize = 4000;

#[derive(Debug, Deserialize)]
struct ReleaseDto {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
    body: Option<String>,
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
}
