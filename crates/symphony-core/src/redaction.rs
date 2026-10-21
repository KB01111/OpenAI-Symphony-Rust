use regex::Regex;
use std::sync::{OnceLock, RwLock};

/// Secrets known only at runtime (API tokens, credential vault values) that the patterns below
/// cannot recognise. Every [`redact_secrets`] call masks them, wherever its output goes (log files,
/// agent stderr events, diagnostics).
static REGISTERED_SECRETS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Minimum length of a value to be redacted, so common short words are never masked.
const MIN_SECRET_LEN: usize = 6;

/// Bound on registered secrets. Each `redact_secrets` call scans all of them, and rotated
/// credentials stay registered, so the oldest registrations are dropped beyond this count (far
/// more than a session's real credentials and tokens).
const MAX_REGISTERED_SECRETS: usize = 512;

/// Make every later [`redact_secrets`] call mask `secret`. Values shorter than 6 characters are
/// ignored.
pub fn register_secret(secret: &str) {
    if let Ok(mut known) = REGISTERED_SECRETS.write() {
        register_in(&mut known, secret, MAX_REGISTERED_SECRETS);
    }
}

/// Add `secret` to `known`, keeping at most `capacity` entries (oldest dropped first).
fn register_in(known: &mut Vec<String>, secret: &str, capacity: usize) {
    let secret = secret.trim();
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    if let Some(index) = known.iter().position(|k| k == secret) {
        // Re-registering refreshes the entry, so live credentials are never the oldest.
        let existing = known.remove(index);
        known.push(existing);
    } else {
        if known.len() >= capacity {
            known.remove(0);
        }
        known.push(secret.to_string());
    }
}

/// Where a too-long `line` may be cut so that redacting each piece separately is as good as
/// redacting the whole line. Returns the index of a space or tab to cut after, or `None` when no
/// such point exists. A cut never falls
/// - between `Bearer` and its token (the bearer pattern needs both), or
/// - inside a registered secret, nor right before the start of one that may continue past the end
///   of `line` (registered secrets can contain whitespace; the other patterns cannot).
pub fn redaction_safe_split(line: &[u8]) -> Option<usize> {
    let is_space = |b: &u8| *b == b' ' || *b == b'\t';
    let registered = REGISTERED_SECRETS.read();
    let spaced: Vec<&[u8]> = registered
        .as_deref()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(String::as_bytes)
        .filter(|s| s.iter().any(is_space))
        .collect();
    let mut end = line.len();
    while let Some(split) = line[..end].iter().rposition(is_space) {
        let cut = split + 1;
        let word_end = line[..split]
            .iter()
            .rposition(|b| !is_space(b))
            .map_or(0, |i| i + 1);
        let word_start = line[..word_end]
            .iter()
            .rposition(is_space)
            .map_or(0, |i| i + 1);
        let after_bearer = line[word_start..word_end].eq_ignore_ascii_case(b"bearer");
        if !after_bearer && !spaced.iter().any(|secret| cuts(line, cut, secret)) {
            return Some(split);
        }
        end = split;
    }
    None
}

/// Whether cutting `line` at `cut` splits an occurrence of `secret`, counting an occurrence that
/// starts before `cut` and runs to the end of `line` (it may continue in the next piece).
fn cuts(line: &[u8], cut: usize, secret: &[u8]) -> bool {
    (cut.saturating_sub(secret.len() - 1)..cut).any(|start| {
        let available = &line[start..];
        let n = secret.len().min(available.len());
        start + secret.len() > cut && available[..n] == secret[..n]
    })
}

/// Redacts common secret tokens, API keys, registered runtime secrets and explicitly supplied
/// secret values from logs and exports.
pub fn redact_secrets(input: &str, extra_secrets: &[String]) -> String {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r"gh[pousr]_[A-Za-z0-9_]{16,255}").expect("valid regex"),
            Regex::new(r"github_pat_[A-Za-z0-9_]{16,255}").expect("valid regex"),
            Regex::new(r"sk-[A-Za-z0-9_-]{16,255}").expect("valid regex"),
            Regex::new(r"lin_api_[A-Za-z0-9_]{16,255}").expect("valid regex"),
            Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-]{16,512}").expect("valid regex"),
        ]
    });

    let mut out = input.to_string();
    let registered = REGISTERED_SECRETS.read();
    let registered: &[String] = registered.as_deref().map(Vec::as_slice).unwrap_or(&[]);
    for secret in extra_secrets.iter().chain(registered) {
        let trimmed = secret.trim();
        if trimmed.len() >= MIN_SECRET_LEN {
            out = out.replace(trimmed, "[REDACTED]");
        }
    }

    for re in patterns {
        out = re.replace_all(&out, "[REDACTED]").into_owned();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_never_separate_bearer_from_its_token() {
        assert_eq!(redaction_safe_split(b"a b"), Some(1));
        // The last whitespace follows "Bearer": cut before "Bearer" instead.
        assert_eq!(redaction_safe_split(b"auth: Bearer  abc"), Some(5));
        assert_eq!(redaction_safe_split(b"x BEARER abc"), Some(1));
        assert_eq!(redaction_safe_split(b"Bearer abc"), None);
        assert_eq!(redaction_safe_split(b"unbroken"), None);
    }

    #[test]
    fn splits_never_cut_a_registered_secret_containing_whitespace() {
        register_secret("open sesame 42 phrase");
        // Whole occurrence around the last space: cut before it.
        assert_eq!(redaction_safe_split(b"log: open sesame 42 phrase"), Some(4));
        // An occurrence that may continue past the end of the buffer is kept whole too.
        assert_eq!(redaction_safe_split(b"log: x open sesame 4"), Some(6));
        assert_eq!(redaction_safe_split(b"open sesame 42"), None);
    }

    #[test]
    fn the_registry_is_bounded_and_keeps_refreshed_entries() {
        // A local registry: filling the global one would evict other tests' secrets.
        let mut known = Vec::new();
        let live = "live-credential-Kp3";
        register_in(&mut known, live, 8);
        for i in 0..100 {
            register_in(&mut known, &format!("rotated-token-{i:06}"), 8);
            if i % 5 == 0 {
                register_in(&mut known, live, 8); // still in use: refreshed, never evicted
            }
        }
        assert_eq!(known.len(), 8);
        assert!(known.iter().any(|k| k == live));
        assert!(
            !known.iter().any(|k| k == "rotated-token-000000"),
            "oldest dropped"
        );
        register_in(&mut known, "short", 8);
        assert!(!known.iter().any(|k| k == "short"), "too short to register");
    }

    #[test]
    fn registered_secrets_are_redacted_everywhere() {
        // An arbitrary provider key matching no pattern.
        let key = "acme-key-Zq81kLm2";
        assert_eq!(
            redact_secrets(&format!("auth {key}"), &[]),
            format!("auth {key}")
        );
        register_secret(key);
        register_secret("abc"); // too short: never registered
        assert_eq!(
            redact_secrets(&format!("auth {key} abc"), &[]),
            "auth [REDACTED] abc"
        );
    }
}
