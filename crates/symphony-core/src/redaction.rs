use regex::Regex;
use std::sync::{OnceLock, RwLock};

/// Secrets known only at runtime (API tokens, credential vault values) that the patterns below
/// cannot recognise. Every [`redact_secrets`] call masks them, wherever its output goes (log files,
/// agent stderr events, diagnostics).
static REGISTERED_SECRETS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Minimum length of a value to be redacted, so common short words are never masked.
const MIN_SECRET_LEN: usize = 6;

/// Make every later [`redact_secrets`] call mask `secret`. Values shorter than 6 characters are
/// ignored.
pub fn register_secret(secret: &str) {
    let secret = secret.trim();
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    if let Ok(mut known) = REGISTERED_SECRETS.write() {
        if !known.iter().any(|k| k == secret) {
            known.push(secret.to_string());
        }
    }
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
