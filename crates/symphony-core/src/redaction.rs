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
/// the oldest retired values are dropped (far more than a session's real rotations). Values in
/// use are never dropped.
const MAX_RETIRED_SECRETS: usize = 512;

/// Registered secrets. `owned` maps each source of a secret in use (a vault key, an environment
/// variable, the local API token) to its current value, so their number is bounded by the sources.
/// A value no source holds any more (rotated, deleted or never stored, possibly still valid at its
/// provider) moves to `retired`, kept up to a bound. The same value may have several owners; it
/// stays in use until the last one releases it.
struct Registry {
    owned: Vec<(String, String)>,
    retired: VecDeque<String>,
}

impl Registry {
    const fn new() -> Self {
        Self {
            owned: Vec::new(),
            retired: VecDeque::new(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &String> {
        self.owned
            .iter()
            .map(|(_, value)| value)
            .chain(&self.retired)
    }

    fn in_use(&self, secret: &str) -> bool {
        self.owned.iter().any(|(_, value)| value == secret)
    }

    fn register(&mut self, owner: &str, secret: &str, capacity: usize) {
        let Some(secret) = significant(secret) else {
            self.release(owner, capacity);
            return;
        };
        self.retired.retain(|k| k != secret);
        match self.owned.iter().position(|(o, _)| o == owner) {
            Some(index) if self.owned[index].1 == secret => {}
            Some(index) => {
                let previous = std::mem::replace(&mut self.owned[index].1, secret.to_string());
                self.retire_unless_in_use(previous, capacity);
            }
            None => self.owned.push((owner.to_string(), secret.to_string())),
        }
    }

    fn release(&mut self, owner: &str, capacity: usize) {
        if let Some(index) = self.owned.iter().position(|(o, _)| o == owner) {
            let (_, previous) = self.owned.remove(index);
            self.retire_unless_in_use(previous, capacity);
        }
    }

    fn retire_unless_in_use(&mut self, secret: String, capacity: usize) {
        if self.in_use(&secret) || self.retired.contains(&secret) {
            return;
        }
        if self.retired.len() >= capacity {
            self.retired.pop_front();
        }
        self.retired.push_back(secret);
    }

    fn forget(&mut self, secret: &str) {
        if !self.in_use(secret) {
            self.retired.retain(|k| k != secret);
        }
    }
}

fn significant(secret: &str) -> Option<&str> {
    let secret = secret.trim();
    (secret.len() >= MIN_SECRET_LEN).then_some(secret)
}

fn registry() -> Option<RwLockReadGuard<'static, Registry>> {
    REGISTERED_SECRETS.read().ok()
}

/// Make every later [`redact_secrets`] call mask `secret`, the current value of `owner` (e.g.
/// `vault:GITHUB_TOKEN`, `env:OPENAI_API_KEY`, `local-api-token`). A value in use is never dropped.
/// The owner's previous value, unless another owner still holds it, is retired: it stays masked
/// (it may still be valid at its provider) until 512 newer values have been retired. Values
/// shorter than 6 characters are ignored (and release the owner's previous value).
pub fn register_secret(owner: &str, secret: &str) {
    if let Ok(mut registry) = REGISTERED_SECRETS.write() {
        registry.register(owner, secret, MAX_RETIRED_SECRETS);
    }
}

/// `owner` no longer holds a secret (e.g. a deleted credential). Its value is retired like a
/// replaced one, unless another owner still holds it.
pub fn release_secret(owner: &str) {
    if let Ok(mut registry) = REGISTERED_SECRETS.write() {
        registry.release(owner, MAX_RETIRED_SECRETS);
    }
}

/// Stop masking `secret` once no owner holds it: only for a value Symphony itself has invalidated
/// (a rotated or revoked remote access token), which is worthless once revoked and must not use up
/// the retired bound that keeps replaced provider credentials masked. A value some owner (another
/// credential, an environment variable, the local API token) still holds stays masked.
pub fn forget_secret(secret: &str) {
    if let (Some(secret), Ok(mut registry)) = (significant(secret), REGISTERED_SECRETS.write()) {
        registry.forget(secret);
    }
}

/// Where a too-long `line` may be cut so that redacting each piece separately is as good as
/// redacting the whole line. Returns the index of a space or tab to cut after, or `None` when no
/// such point exists. A cut never falls
/// - between `Bearer` (also at the end of a word) and its token (the bearer pattern needs both), or
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
        // The bearer pattern has no word boundary: "Authorization:Bearer <token>" matches too.
        let word = &line[word_start..word_end];
        let after_bearer =
            word.len() >= 6 && word[word.len() - 6..].eq_ignore_ascii_case(b"bearer");
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
        assert_eq!(redaction_safe_split(b"x Authorization:Bearer abc"), Some(1));
        assert_eq!(redaction_safe_split(b"Authorization:bearer abc"), None);
        assert_eq!(redaction_safe_split(b"Bearer abc"), None);
        assert_eq!(redaction_safe_split(b"unbroken"), None);
    }

    #[test]
    fn splits_never_cut_a_registered_secret_containing_whitespace() {
        register_secret("test:spaced", "open sesame 42 phrase");
        // Whole occurrence around the last space: cut before it.
        assert_eq!(redaction_safe_split(b"log: open sesame 42 phrase"), Some(4));
        // An occurrence that may continue past the end of the buffer is kept whole too.
        assert_eq!(redaction_safe_split(b"log: x open sesame 4"), Some(6));
        assert_eq!(redaction_safe_split(b"open sesame 42"), None);
    }

    #[test]
    fn values_in_use_are_never_evicted_and_retired_ones_are_bounded() {
        // A local registry: filling the global one would evict other tests' secrets.
        let mut registry = Registry::new();
        registry.register("vault:PROVIDER", "live-credential-Kp3", 8);
        // E.g. remote token rotations: each replaced token is retired.
        for i in 0..1000 {
            registry.register("vault:REMOTE", &format!("rotated-token-{i:06}"), 8);
        }
        let names: Vec<&str> = registry.iter().map(String::as_str).collect();
        assert!(
            names.contains(&"live-credential-Kp3"),
            "in use: never evicted"
        );
        assert!(names.contains(&"rotated-token-000999"), "current token");
        assert_eq!(registry.retired.len(), 8, "retired values are bounded");
        assert!(
            names.contains(&"rotated-token-000998"),
            "newest retired kept"
        );
        assert!(!names.contains(&"rotated-token-000000"), "oldest dropped");

        // A retired value that comes back into use is in use again, not duplicated.
        registry.register("vault:OTHER", "rotated-token-000998", 8);
        assert!(registry.in_use("rotated-token-000998"));
        assert!(!registry.retired.iter().any(|k| k == "rotated-token-000998"));
        registry.register("vault:SHORT", "short", 8);
        assert!(
            !registry.iter().any(|k| k == "short"),
            "too short to register"
        );
    }

    #[test]
    fn a_value_with_several_owners_stays_in_use_until_the_last_releases_it() {
        let mut registry = Registry::new();
        let shared = "shared-value-Hq8Vb2";
        registry.register("vault:REMOTE", shared, 8);
        registry.register("env:PROVIDER_KEY", shared, 8);
        registry.release("vault:REMOTE", 8);
        registry.forget(shared);
        assert!(registry.in_use(shared), "still held by the environment");
        registry.release("env:PROVIDER_KEY", 8);
        assert!(
            registry.retired.iter().any(|k| k == shared),
            "retired, still masked"
        );
        registry.forget(shared);
        assert!(
            !registry.iter().any(|k| k == shared),
            "forgotten once unowned"
        );
    }

    #[test]
    fn forgotten_secrets_are_no_longer_masked() {
        let revoked = "revoked-remote-token-Q7w";
        register_secret("test:forget", revoked);
        assert_eq!(redact_secrets(revoked, &[]), "[REDACTED]");
        forget_secret(revoked);
        assert_eq!(redact_secrets(revoked, &[]), "[REDACTED]", "still in use");
        release_secret("test:forget");
        forget_secret(revoked);
        assert_eq!(redact_secrets(revoked, &[]), revoked);
    }

    #[test]
    fn registered_secrets_are_redacted_everywhere() {
        // An arbitrary provider key matching no pattern.
        let key = "acme-key-Zq81kLm2";
        assert_eq!(
            redact_secrets(&format!("auth {key}"), &[]),
            format!("auth {key}")
        );
        register_secret("test:key", key);
        register_secret("test:short", "abc"); // too short: never registered
        assert_eq!(
            redact_secrets(&format!("auth {key} abc"), &[]),
            "auth [REDACTED] abc"
        );
    }
}
