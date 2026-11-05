//! Which OpenCode releases the OpenCode driver speaks to.
//!
//! The driver is written against the OpenCode 1.x server API (`/session`, `/event`,
//! `/session/{id}/prompt_async`, `/permission/{id}/reply`, `/session/{id}/abort`) and was verified
//! against 1.19.4. OpenCode 2 serves other routes (`POST /api/session`, responses wrapped in
//! `data`), so it is rejected up front with a clear error instead of failing later in a request.
//!
//! Only a version that is *known* to be outside the range blocks a run. A binary that prints no
//! recognisable version (a wrapper script, a fork) is let through: the range check is there to stop
//! a known-incompatible release, not to second-guess every launcher.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use symphony_core::error::{Result, SymphonyError};

/// First release the driver is verified against.
pub const MIN_SUPPORTED: (u64, u64, u64) = (1, 19, 4);
/// First release that is *not* supported (OpenCode 2).
pub const MAX_EXCLUSIVE: (u64, u64, u64) = (2, 0, 0);
/// Human-readable form of the supported range, for messages and docs.
pub const SUPPORTED_RANGE: &str = ">=1.19.4, <2.0.0";

/// Error code of the run-time refusal.
pub const UNSUPPORTED_CODE: &str = "opencode_version_unsupported";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSupport {
    Supported,
    /// Older than [`MIN_SUPPORTED`].
    TooOld,
    /// [`MAX_EXCLUSIVE`] or newer (OpenCode 2).
    TooNew,
    /// No `x.y.z` in the text; not blocked (see the module documentation).
    Unparseable,
}

impl VersionSupport {
    /// Whether a run may start.
    pub fn is_usable(self) -> bool {
        matches!(self, Self::Supported | Self::Unparseable)
    }
}

/// A version this large is a date or a build number (`2026.11.5`), not an OpenCode release.
const MAX_PLAUSIBLE_MAJOR: u64 = 99;

/// The first `x.y.z` in `text` and whether it carries a pre-release suffix, tolerating `v`,
/// `opencode 1.19.4`, `opencode/1.19.4`, `1.20.0-beta.1` and `1.20.0+abc`.
fn parse_full(text: &str) -> Option<((u64, u64, u64), bool)> {
    text.split(|c: char| c.is_whitespace() || c == '/' || c == '(' || c == ')')
        .find_map(|word| {
            let word = word.trim_start_matches(['v', 'V']);
            let end = word
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(word.len());
            let (number, rest) = word.split_at(end);
            let mut parts = number.trim_matches('.').split('.');
            let major: u64 = parts.next()?.parse().ok()?;
            let minor = parts.next()?.parse().ok()?;
            let patch = parts.next()?.parse().ok()?;
            if parts.next().is_some() || major > MAX_PLAUSIBLE_MAJOR {
                return None;
            }
            Some(((major, minor, patch), rest.starts_with('-')))
        })
}

/// The first `x.y.z` in `text` (see [`parse_full`]); a date or build number is not a version.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    parse_full(text).map(|(version, _)| version)
}

pub fn check(text: &str) -> VersionSupport {
    match parse_full(text) {
        None => VersionSupport::Unparseable,
        // A pre-release of the floor (1.19.4-beta.1) is older than the floor itself.
        Some((v, prerelease)) if v < MIN_SUPPORTED || (v == MIN_SUPPORTED && prerelease) => {
            VersionSupport::TooOld
        }
        // 2.0.0-beta already serves the OpenCode 2 API.
        Some((v, _)) if v >= MAX_EXCLUSIVE => VersionSupport::TooNew,
        Some(_) => VersionSupport::Supported,
    }
}

/// The one wording used by provider detection and the run-time error. `None` when usable.
pub fn unsupported_message(text: &str) -> Option<String> {
    let advice = match check(text) {
        VersionSupport::Supported | VersionSupport::Unparseable => return None,
        VersionSupport::TooOld => "Update OpenCode to a 1.x release of at least 1.19.4.",
        VersionSupport::TooNew => {
            "OpenCode 2 uses a different server API; install a 1.x release (npm install -g opencode-ai@1)."
        }
    };
    Some(format!(
        "OpenCode {} is not supported; Symphony needs OpenCode {SUPPORTED_RANGE}. {advice}",
        text.trim()
    ))
}

/// Probe `exe --version` and refuse a known-incompatible release.
///
/// `exe` is the configured command, resolved the way the launch resolves it: a bare name on `PATH`,
/// a relative path against the task `workspace` (the launch runs there), anything else as given. A
/// probe that fails or prints nothing recognisable does not block the run.
pub(crate) async fn ensure_supported(exe: &str, workspace: &Path) -> Result<()> {
    let path = if exe.contains(['/', '\\']) {
        let given = Path::new(exe);
        Some(if given.is_absolute() {
            given.to_path_buf()
        } else {
            workspace.join(given)
        })
    } else {
        crate::detection::find_on_path(exe)
    };
    let Some(path) = path else {
        return Ok(()); // the launch reports "not found" with its own error
    };
    // A binary that passed once is not probed again until it changes on disk.
    let stamp = std::fs::metadata(&path)
        .and_then(|m| m.modified().map(|modified| (m.len(), modified)))
        .ok()
        .map(|(len, modified)| (path.clone(), len, modified));
    static VERIFIED: OnceLock<Mutex<HashSet<(PathBuf, u64, SystemTime)>>> = OnceLock::new();
    let verified = VERIFIED.get_or_init(Default::default);
    if let Some(stamp) = &stamp {
        if verified.lock().is_ok_and(|set| set.contains(stamp)) {
            return Ok(());
        }
    }
    let Some(version) = crate::detection::probe_version(&path).await else {
        return Ok(());
    };
    match unsupported_message(&version) {
        None => {
            if let (Some(stamp), Ok(mut set)) = (stamp, verified.lock()) {
                set.insert(stamp);
            }
            Ok(())
        }
        Some(message) => Err(SymphonyError::AgentProtocolError {
            code: UNSUPPORTED_CODE.into(),
            message,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_common_version_outputs() {
        assert_eq!(parse_version("1.19.4"), Some((1, 19, 4)));
        assert_eq!(parse_version("v1.19.4\n"), Some((1, 19, 4)));
        assert_eq!(parse_version("opencode 1.19.4"), Some((1, 19, 4)));
        assert_eq!(parse_version("opencode/2.0.1"), Some((2, 0, 1)));
        assert_eq!(parse_version("1.20.0-beta.1"), Some((1, 20, 0)));
        assert_eq!(parse_version("1.20.0+abc"), Some((1, 20, 0)));
        assert_eq!(
            parse_version("OpenCode CLI 1.19.4 (linux)"),
            Some((1, 19, 4))
        );
        assert_eq!(parse_version("1.19"), None);
        // A date or build number is not a release; neither is a lone `v`-less word.
        assert_eq!(parse_version("2026.11.5"), None);
        assert_eq!(parse_version("wrapper 2026.11.05 build"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("dev"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn the_range_is_1_19_4_up_to_but_not_including_2() {
        assert_eq!(check("1.19.4"), VersionSupport::Supported);
        assert_eq!(check("1.19.5"), VersionSupport::Supported);
        assert_eq!(check("1.20.0"), VersionSupport::Supported);
        assert_eq!(check("1.99.99"), VersionSupport::Supported);
        assert_eq!(check("1.19.3"), VersionSupport::TooOld);
        assert_eq!(check("1.0.0"), VersionSupport::TooOld);
        assert_eq!(check("0.15.2"), VersionSupport::TooOld);
        assert_eq!(check("2.0.0"), VersionSupport::TooNew);
        assert_eq!(check("opencode/2.0.1"), VersionSupport::TooNew);
        assert_eq!(check("3.1.0"), VersionSupport::TooNew);
        assert_eq!(check("something else"), VersionSupport::Unparseable);
    }

    #[test]
    fn a_prerelease_is_older_than_its_release() {
        assert_eq!(check("1.19.4-beta.1"), VersionSupport::TooOld);
        assert_eq!(check("opencode 1.19.4-rc.2"), VersionSupport::TooOld);
        assert_eq!(check("1.19.5-beta.1"), VersionSupport::Supported);
        assert_eq!(check("1.20.0-beta"), VersionSupport::Supported);
        // 2.0.0-beta already has the OpenCode 2 API.
        assert_eq!(check("2.0.0-beta.1"), VersionSupport::TooNew);
        assert_eq!(check("2026.11.5"), VersionSupport::Unparseable);
    }

    #[test]
    fn only_a_known_incompatible_version_blocks() {
        assert!(check("1.19.4").is_usable());
        assert!(check("not a version").is_usable());
        assert!(!check("2.0.0").is_usable());
        assert!(!check("1.18.0").is_usable());
    }

    #[test]
    fn the_message_names_the_version_the_range_and_the_remedy() {
        assert_eq!(unsupported_message("1.19.4"), None);
        assert_eq!(unsupported_message("garbage"), None);
        let v2 = unsupported_message("2.0.1").unwrap();
        assert!(v2.contains("OpenCode 2.0.1 is not supported"), "{v2}");
        assert!(v2.contains(SUPPORTED_RANGE), "{v2}");
        assert!(v2.contains("1.x"), "{v2}");
        let old = unsupported_message("1.18.0").unwrap();
        assert!(old.contains("1.19.4"), "{old}");
    }
}
