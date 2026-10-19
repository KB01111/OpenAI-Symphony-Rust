use regex::Regex;
use std::sync::OnceLock;

/// Redacts common secret tokens, API keys, and explicitly supplied secret values from logs and exports.
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
    for secret in extra_secrets {
        let trimmed = secret.trim();
        if trimmed.len() >= 6 {
            out = out.replace(trimmed, "[REDACTED]");
        }
    }

    for re in patterns {
        out = re.replace_all(&out, "[REDACTED]").into_owned();
    }

    out
}
