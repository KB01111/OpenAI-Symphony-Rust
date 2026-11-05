//! Which model-service keys an OpenCode process may inherit.
//!
//! A run through a connection profile gets exactly one service key (the profile's) in
//! `extra_env`. Keys of the *other* services, and the OpenAI and Anthropic keys, that happen to be
//! set on the host are removed from that run, so a task cannot read a key the profile never asked
//! for. A run with no profile is left alone: an ambient key keeps working for OpenCode as before.

use std::collections::HashMap;

use symphony_core::ModelService;
use tokio::process::Command;

/// The provider keys Symphony passes to agents outside connection profiles.
const PROVIDER_KEY_ENV: [&str; 2] = ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"];

/// Names of the environment variables to remove for a run with `extra_env`: every service key
/// variable that `extra_env` does not set, but only when it sets one (a profile run).
pub(crate) fn unselected_service_keys(extra_env: &HashMap<String, String>) -> Vec<&'static str> {
    let services = ModelService::ALL.map(ModelService::key_env_name);
    if !services.iter().any(|k| extra_env.contains_key(*k)) {
        return Vec::new();
    }
    services
        .into_iter()
        .chain(PROVIDER_KEY_ENV)
        .filter(|k| !extra_env.contains_key(*k))
        .collect()
}

/// Remove the service keys a profile run must not inherit (see [`unselected_service_keys`]).
pub(crate) fn scrub_unselected_service_keys(
    cmd: &mut Command,
    extra_env: &HashMap<String, String>,
) {
    for key in unselected_service_keys(extra_env) {
        cmd.env_remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[&str]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|k| (k.to_string(), "v".to_string()))
            .collect()
    }

    #[test]
    fn a_profile_run_keeps_only_its_own_service_key() {
        let mut removed =
            unselected_service_keys(&env(&["OPENROUTER_API_KEY", "OPENCODE_CONFIG_CONTENT"]));
        removed.sort_unstable();
        assert_eq!(
            removed,
            [
                "ANTHROPIC_API_KEY",
                "NVIDIA_API_KEY",
                "OPENAI_API_KEY",
                "OPENCODE_API_KEY"
            ]
        );
        let mut go = unselected_service_keys(&env(&["OPENCODE_API_KEY"]));
        go.sort_unstable();
        assert_eq!(
            go,
            [
                "ANTHROPIC_API_KEY",
                "NVIDIA_API_KEY",
                "OPENAI_API_KEY",
                "OPENROUTER_API_KEY"
            ]
        );
    }

    #[test]
    fn a_run_without_a_profile_is_left_alone() {
        assert!(unselected_service_keys(&HashMap::new()).is_empty());
        // Provider keys and unrelated variables alone do not make it a profile run.
        assert!(unselected_service_keys(&env(&["OPENAI_API_KEY", "FOO"])).is_empty());
    }
}
