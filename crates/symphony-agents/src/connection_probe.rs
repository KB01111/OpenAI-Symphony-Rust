//! Asks a model service which models a key can use. Backs both the connection test and model
//! discovery of a connection profile, so they cannot disagree.
//!
//! The key is sent only to the URL the caller resolved (see `symphony_core::resolve_connection`,
//! which restricts it to the service's own host). Redirects are not followed, the wait is bounded,
//! and nothing from the response body reaches an error message.

use std::time::Duration;

use serde_json::Value;
use symphony_core::{ProfileStatus, ResolvedConnection};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MODELS: usize = 2_000;

/// What a probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub status: ProfileStatus,
    /// One line for the user (no secrets, no response body).
    pub detail: String,
    /// Model ids as the service names them, sorted and unique; empty unless the list was read.
    pub models: Vec<String>,
}

impl ProbeResult {
    fn new(status: ProfileStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
            models: Vec::new(),
        }
    }
}

/// Model ids in the shapes services use: `{"data":[{"id":…}]}` (OpenAI-compatible),
/// `{"models":[…]}` or a bare array, of objects with an `id` or `name`, or of strings.
pub fn parse_model_ids(body: &Value) -> Vec<String> {
    let items = body
        .get("data")
        .or_else(|| body.get("models"))
        .unwrap_or(body)
        .as_array();
    let mut ids: Vec<String> = items
        .into_iter()
        .flatten()
        .filter_map(|item| match item {
            Value::String(id) => Some(id.as_str()),
            other => other
                .get("id")
                .or_else(|| other.get("name"))
                .and_then(Value::as_str),
        })
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= 200)
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    ids.truncate(MAX_MODELS);
    ids
}

/// Probe `conn` with `api_key`.
pub async fn probe_models(conn: &ResolvedConnection, api_key: &str) -> ProbeResult {
    probe_models_url(&conn.models_url(), api_key).await
}

/// Probe a models URL. Callers outside tests go through [`probe_models`].
pub async fn probe_models_url(url: &str, api_key: &str) -> ProbeResult {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(_) => return ProbeResult::new(ProfileStatus::Error, "could not create an HTTP client"),
    };
    let response = match client
        .get(url)
        .bearer_auth(api_key)
        .header("accept", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) if e.is_timeout() => {
            return ProbeResult::new(ProfileStatus::Error, "the service did not answer in time")
        }
        Err(_) => return ProbeResult::new(ProfileStatus::Error, "could not reach the service"),
    };
    let code = response.status().as_u16();
    match code {
        200..=299 => {}
        401 | 403 => {
            return ProbeResult::new(
                ProfileStatus::Error,
                format!("the service refused the key (HTTP {code})"),
            )
        }
        // The service answers but offers no model list: the connection may still work.
        404 | 405 | 501 => {
            return ProbeResult::new(
                ProfileStatus::Limited,
                format!("the service has no model list (HTTP {code}); enter a model id by hand"),
            )
        }
        429 => {
            return ProbeResult::new(
                ProfileStatus::Limited,
                "the service is rate limiting this key (HTTP 429)",
            )
        }
        300..=399 => {
            return ProbeResult::new(
                ProfileStatus::Error,
                format!(
                    "the service redirected the request (HTTP {code}); redirects are not followed"
                ),
            )
        }
        _ => {
            return ProbeResult::new(
                ProfileStatus::Error,
                format!("the service answered HTTP {code}"),
            )
        }
    }
    let body = match response.json::<Value>().await {
        Ok(body) => body,
        Err(_) => {
            return ProbeResult::new(
                ProfileStatus::Limited,
                "the service answered, but not with a model list",
            )
        }
    };
    let models = parse_model_ids(&body);
    if models.is_empty() {
        return ProbeResult::new(ProfileStatus::Limited, "the service listed no models");
    }
    ProbeResult {
        status: ProfileStatus::Verified,
        detail: format!("{} models", models.len()),
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_ids_are_read_from_the_common_shapes() {
        let openai =
            json!({ "data": [{ "id": "b" }, { "id": "a" }, { "id": "a" }, { "object": "x" }] });
        assert_eq!(parse_model_ids(&openai), ["a", "b"]);
        assert_eq!(
            parse_model_ids(&json!({ "models": [{ "name": "m1" }, "m2"] })),
            ["m1", "m2"]
        );
        assert_eq!(parse_model_ids(&json!(["z", " y "])), ["y", "z"]);
        assert!(parse_model_ids(&json!({ "data": "nope" })).is_empty());
        assert!(parse_model_ids(&json!(null)).is_empty());
    }
}
