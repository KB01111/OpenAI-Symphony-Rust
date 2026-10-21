use regex::Regex;
use std::collections::VecDeque;
use std::sync::{OnceLock, RwLock, RwLockReadGuard};

/// Secrets known only at runtime (API tokens, credential vault values) that the patterns below
/// cannot recognise. Every [`redact_secrets`] call masks them, wherever its output goes (log files,
/// agent stderr events, diagnostics).
static REGISTERED_SECRETS: RwLock<Registry> = RwLock::new(Registry::new());

/// Minimum length of a value to be redacted, so common short words are never masked.
const MIN_SECRET_LEN: usize = 6;

/// Bound on retired secrets. Each `redact_secrets` call scans all of them, so beyond this count
/// the oldest retired values are dropped (far more than a session's real rotations). Live secrets
/// are never dropped.
const MAX_RETIRED_SECRETS: usize = 512;

/// Registered secrets: `live` ones are in use and never evicted; each is retired by its owner when
/// replaced, so their number stays bounded by what is in use. `retired` ones (rotated or deleted
/// credentials, which may remain valid at their provider) are kept up to a bound.
struct Registry {
    live: Vec<String>,
    retired: VecDeque<String>,
}

impl Registry {
    const fn new() -> Self {
        Self {
            live: Vec::new(),
            retired: VecDeque::new(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &String> {
        self.live.iter().chain(&self.retired)
    }

    fn register(&mut self, secret: &str) {
        let Some(secret) = significant(secret) else {
            return;
        };
        if let Some(index) = self.retired.iter().position(|k| k == secret) {
            self.retired.remove(index);
        }
        if !self.live.iter().any(|k| k == secret) {
            self.live.push(secret.to_string());
        }
    }

    fn retire(&mut self, secret: &str, capacity: usize) {
        let Some(secret) = significant(secret) else {
            return;
        };
        self.live.retain(|k| k != secret);
        if let Some(index) = self.retired.iter().position(|k| k == secret) {
            self.retired.remove(index);
        } else if self.retired.len() >= capacity {
            self.retired.pop_front();
        }
        self.retired.push_back(secret.to_string());
    }
}

fn significant(secret: &str) -> Option<&str> {
    let secret = secret.trim();
    (secret.len() >= MIN_SECRET_LEN).then_some(secret)
}

fn registry() -> Option<RwLockReadGuard<'static, Registry>> {
    REGISTERED_SECRETS.read().ok()
}

/// Make every later [`redact_secrets`] call mask `secret`, a value in use (a live credential or
/// API token) that is never dropped. Whoever replaces it calls [`retire_secret`]. Values shorter
/// than 6 characters are ignored.
pub fn register_secret(secret: &str) {
    if let Ok(mut registry) = REGISTERED_SECRETS.write() {
        registry.register(secret);
    }
}

/// Mark a registered secret as no longer in use (rotated, deleted, or never stored). It stays
/// masked, possibly still being valid at its provider, until 512 newer values have been retired.
pub fn retire_secret(secret: &str) {
    if let Ok(mut registry) = REGISTERED_SECRETS.write() {
        registry.retire(secret, MAX_RETIRED_SECRETS);
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
    let registered = registry();
    let spaced: Vec<&[u8]> = registered
        .iter()
        .flat_map(|registry| registry.iter())
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
    let registered = registry();
    let registered = registered.iter().flat_map(|registry| registry.iter());
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
    fn live_secrets_are_never_evicted_and_retired_ones_are_bounded() {
        // A local registry: filling the global one would evict other tests' secrets.
        let mut registry = Registry::new();
        let live = "live-credential-Kp3";
        registry.register(live);
        // E.g. remote token rotations: each new token is live, the replaced one retired.
        for i in 0..1000 {
            registry.register(&format!("rotated-token-{i:06}"));
            if i > 0 {
                registry.retire(&format!("rotated-token-{:06}", i - 1), 8);
            }
        }
        let names: Vec<&str> = registry.iter().map(String::as_str).collect();
        assert_eq!(
            registry.live,
            ["live-credential-Kp3", "rotated-token-000999"]
        );
        assert_eq!(registry.retired.len(), 8, "retired values are bounded");
        assert!(
            names.contains(&"rotated-token-000998"),
            "newest retired kept"
        );
        assert!(!names.contains(&"rotated-token-000000"), "oldest dropped");

        // A retired value that comes back into use is live again, not duplicated.
        registry.register("rotated-token-000998");
        assert!(registry.live.iter().any(|k| k == "rotated-token-000998"));
        assert!(!registry.retired.iter().any(|k| k == "rotated-token-000998"));
        registry.register("short");
        assert!(
            !registry.iter().any(|k| k == "short"),
            "too short to register"
        );
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
