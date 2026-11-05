//! Connection profile endpoints: validation, the connection test, the credential allowlist.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use symphony_core::{ProfileStatus, ResolvedConnection};
use symphony_orchestrator::{ModelLister, ProbeResult, SymphonyOrchestrator};
use symphony_platform::SecretStore;
use symphony_server::{build_router, ServerConfig, ALLOWED_CREDENTIAL_KEYS};
use symphony_storage::Storage;
use tower::ServiceExt;

const TOKEN: &str = "test-token-0123456789abcdef";
const KEY: &str = "sk-or-endpoint-test-0123456789";

/// A service that accepts every key, and says so in a way that echoes it.
struct EchoingLister;

#[async_trait]
impl ModelLister for EchoingLister {
    async fn list(&self, _: &ResolvedConnection, api_key: &str) -> ProbeResult {
        ProbeResult {
            status: ProfileStatus::Verified,
            detail: format!("accepted {api_key}"),
            models: vec![
                "openai/gpt-5.5".into(),
                "anthropic/claude-sonnet-5.5".into(),
            ],
        }
    }
}

async fn app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let orch = Arc::new(
        SymphonyOrchestrator::new(
            Storage::connect_in_memory().await.unwrap(),
            None,
            SecretStore::new(dir.path()).unwrap(),
        )
        .await
        .unwrap(),
    );
    orch.set_model_lister(Arc::new(EchoingLister));
    let config = ServerConfig {
        default_workspace_root: Some(dir.path().join("workspaces")),
        ..ServerConfig::local(0, Some(TOKEN.to_string()))
    };
    (build_router(orch, &config), dir)
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(format!("/api/v1{uri}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let request = match body {
        Some(b) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn input(name: &str) -> Value {
    json!({ "name": name, "service": "openrouter", "model": "anthropic/claude-sonnet-5.5", "is_default": true })
}

#[tokio::test]
async fn the_service_keys_are_allowed_and_stay_out_of_the_process_environment() {
    for key in [
        "OPENROUTER_API_KEY",
        "OPENCODE_GO_API_KEY",
        "NVIDIA_API_KEY",
    ] {
        assert!(ALLOWED_CREDENTIAL_KEYS.contains(&key), "{key}");
    }
    let (app, _dir) = app().await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/credentials",
        Some(json!({ "key": "OPENROUTER_API_KEY", "value": KEY })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(std::env::var("OPENROUTER_API_KEY").is_err());
    let (_, listed) = call(&app, Method::GET, "/credentials", None).await;
    assert!(listed["configured"]
        .as_array()
        .unwrap()
        .contains(&json!("OPENROUTER_API_KEY")));
    assert!(!listed.to_string().contains(KEY));
    // The allowlist is still closed.
    let (status, body) = call(
        &app,
        Method::POST,
        "/credentials",
        Some(json!({ "key": "SOME_OTHER_API_KEY", "value": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "credential_key_not_allowed");
}

#[tokio::test]
async fn a_profile_is_created_tested_changed_and_deleted() {
    let (app, _dir) = app().await;
    let (status, created) = call(
        &app,
        Method::POST,
        "/connection-profiles",
        Some(input("Main")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["status"], "configured");
    assert_eq!(created["credential_ref"], "OPENROUTER_API_KEY");
    assert_eq!(created["provider"], "opencode");

    // No key yet: the test reports it as a status, not as a server error.
    let (status, tested) = call(
        &app,
        Method::POST,
        &format!("/connection-profiles/{id}/test"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tested["profile"]["status"], "error");
    assert!(tested["profile"]["status_detail"]
        .as_str()
        .unwrap()
        .contains("OPENROUTER_API_KEY"));

    call(
        &app,
        Method::POST,
        "/credentials",
        Some(json!({ "key": "OPENROUTER_API_KEY", "value": KEY })),
    )
    .await;
    let (_, tested) = call(
        &app,
        Method::POST,
        &format!("/connection-profiles/{id}/test"),
        None,
    )
    .await;
    assert_eq!(tested["profile"]["status"], "verified");
    assert_eq!(
        tested["models"],
        json!(["openai/gpt-5.5", "anthropic/claude-sonnet-5.5"])
    );
    // The key never comes back, not even when the service echoes it.
    assert!(!tested.to_string().contains(KEY), "{tested}");

    let (_, models) = call(
        &app,
        Method::GET,
        &format!("/connection-profiles/{id}/models"),
        None,
    )
    .await;
    assert_eq!(models["status"], "verified");
    assert_eq!(models["models"].as_array().unwrap().len(), 2);

    // Replacing the key makes the earlier result stale.
    call(
        &app,
        Method::POST,
        "/credentials",
        Some(json!({ "key": "OPENROUTER_API_KEY", "value": "sk-or-replaced-0123456789" })),
    )
    .await;
    let (_, listed) = call(&app, Method::GET, "/connection-profiles", None).await;
    assert_eq!(listed[0]["status"], "configured");

    let (status, updated) = call(
        &app,
        Method::PUT,
        &format!("/connection-profiles/{id}"),
        Some(json!({ "name": "Renamed", "service": "openrouter" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["model"], Value::Null);

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/connection-profiles/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(
        &app,
        Method::DELETE,
        &format!("/connection-profiles/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (_, listed) = call(&app, Method::GET, "/connection-profiles", None).await;
    assert_eq!(listed, json!([]));
}

#[tokio::test]
async fn a_profile_that_could_send_the_key_elsewhere_is_refused() {
    let (app, _dir) = app().await;
    for base_url in [
        "https://evil.example.com/v1",
        "http://openrouter.ai/api/v1",
        "https://openrouter.ai.evil.com/api/v1",
        "https://user:pw@openrouter.ai/api/v1",
        "https://integrate.api.nvidia.com/v1",
    ] {
        let (status, body) = call(
            &app,
            Method::POST,
            "/connection-profiles",
            Some(json!({ "name": "X", "service": "openrouter", "base_url": base_url })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{base_url}: {body}");
    }
    let (status, _) = call(
        &app,
        Method::POST,
        "/connection-profiles",
        Some(json!({ "name": "X", "service": "not-a-service" })),
    )
    .await;
    assert!(status.is_client_error());
    let (status, _) = call(
        &app,
        Method::POST,
        "/connection-profiles",
        Some(json!({ "name": "X", "service": "openrouter", "model": "a model; rm -rf" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, listed) = call(&app, Method::GET, "/connection-profiles", None).await;
    assert_eq!(listed, json!([]));
}

#[tokio::test]
async fn the_endpoints_need_the_token() {
    let (app, _dir) = app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/connection-profiles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
