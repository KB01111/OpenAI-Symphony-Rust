//! Which model-service keys an OpenCode process may inherit.
//!
//! A run through a connection profile gets exactly one service key (the profile's) in
//! `extra_env`. Keys of the *other* services that happen to be set on the host are removed from
//! that run, so a task cannot read a key the profile never asked for. A run with no profile is
//! left alone: like `OPENAI_API_KEY`, an ambient service key keeps working for OpenCode as before.

use std::collections::HashMap;

use symphony_core::ModelService;
use tokio::process::Command;

/// Names of the environment variables to remove for a run with `extra_env`: every service key
/// variable that `extra_env` does not set, but only when it sets one (a profile run).
pub(crate) fn unselected_service_keys(extra_env: &HashMap<String, String>) -> Vec<&'static str> {
    let keys = ModelService::ALL.map(ModelService::key_env_name);
    if !keys.iter().any(|k| extra_env.contains_key(*k)) {
        return Vec::new();
    }
    keys.into_iter()
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
        assert_eq!(removed, ["NVIDIA_API_KEY", "OPENCODE_API_KEY"]);
        let mut go = unselected_service_keys(&env(&["OPENCODE_API_KEY"]));
        go.sort_unstable();
        assert_eq!(go, ["NVIDIA_API_KEY", "OPENROUTER_API_KEY"]);
    }

    #[test]
    fn a_run_without_a_profile_is_left_alone() {
        assert!(unselected_service_keys(&HashMap::new()).is_empty());
        // Provider keys and unrelated variables do not make it a profile run.
        assert!(unselected_service_keys(&env(&["OPENAI_API_KEY", "FOO"])).is_empty());
    }
}
