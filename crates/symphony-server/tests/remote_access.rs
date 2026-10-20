//! Remote companion access: token lifecycle, local/remote authorization boundary, CORS, transport
//! checks and live listener reconfiguration. Uses real TCP listeners on loopback.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use symphony_core::domain::AppSettings;
use symphony_orchestrator::SymphonyOrchestrator;
use symphony_platform::SecretStore;
use symphony_server::{
    build_router, remote::remote_operation_allowed, RemoteAccessManager, RemoteListenerState,
    ServerConfig, REMOTE_OPERATIONS, REMOTE_TOKEN_KEY,
};
use symphony_storage::Storage;
use tower::ServiceExt;

const LOCAL_TOKEN: &str = "local-token-0123456789abcdef0123456789";

struct Harness {
    orchestrator: Arc<SymphonyOrchestrator>,
    remote: Arc<RemoteAccessManager>,
    local: axum::Router,
    http: reqwest::Client,
    _dir: tempfile::TempDir,
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::connect_in_memory().await.unwrap();
    let secrets = SecretStore::new(dir.path()).unwrap();
    let orchestrator = Arc::new(
        SymphonyOrchestrator::new(storage, None, secrets)
            .await
            .unwrap(),
    );
    let remote = RemoteAccessManager::new(orchestrator.clone(), Some(dir.path().join("ws")));
    // Any port the remote listener will not use (it binds an ephemeral port in these tests).
    remote.set_local_port(1);
    let local = build_router(
        orchestrator.clone(),
        &ServerConfig {
            remote_access: Some(remote.clone()),
            ..ServerConfig::local(0, Some(LOCAL_TOKEN.to_string()))
        },
    );
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    Harness {
        orchestrator,
        remote,
        local,
        http,
        _dir: dir,
    }
}

fn enabled_settings(port: u16) -> AppSettings {
    AppSettings {
        remote_api_enabled: true,
        remote_bind_address: "127.0.0.1".into(),
        remote_port: port,
        ..AppSettings::default()
    }
}

impl Harness {
    async fn save_settings(&self, settings: &AppSettings) {
        self.orchestrator
            .storage()
            .save_settings(settings)
            .await
            .unwrap();
    }

    /// Enable remote access on an ephemeral port and issue a token. Returns (base URL, token).
    async fn start(&self) -> (String, String) {
        self.save_settings(&enabled_settings(0)).await;
        let issued = self.remote.rotate_token().await.unwrap();
        assert_eq!(issued.status.state, RemoteListenerState::Running);
        let addr = issued.status.listening_on.clone().unwrap();
        (format!("http://{addr}"), issued.token)
    }

    async fn local_request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {LOCAL_TOKEN}"));
        let request = match body {
            Some(b) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = self.local.clone().oneshot(request).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

async fn get_status(
    http: &reqwest::Client,
    url: String,
    token: Option<&str>,
) -> reqwest::StatusCode {
    let mut req = http.get(url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap().status()
}

#[test]
fn every_operation_is_classified_and_documented_in_the_contract() {
    let doc = serde_json::to_value(symphony_server::openapi()).unwrap();
    let mut documented = Vec::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        let relative = path.strip_prefix("/api/v1").unwrap();
        for method in ["get", "put", "post", "delete", "patch"] {
            if let Some(op) = item.get(method) {
                let m = Method::from_bytes(method.to_uppercase().as_bytes()).unwrap();
                let allowed = remote_operation_allowed(&m, relative);
                assert_eq!(
                    op["x-symphony-remote"],
                    json!(allowed),
                    "{method} {path}: x-symphony-remote must match the remote scope"
                );
                documented.push((m.to_string(), relative.to_string()));
            }
        }
    }
    // Every remote operation (except the undocumented compatibility aliases) exists.
    for (method, path) in REMOTE_OPERATIONS {
        if *path == "/tasks/id/{id}" || *path == "/{issue_identifier}" {
            continue;
        }
        assert!(
            documented.contains(&(method.to_string(), path.to_string())),
            "{method} {path} is in REMOTE_OPERATIONS but not in the API"
        );
    }
}

#[tokio::test]
async fn remote_access_is_disabled_by_default_and_needs_a_token() {
    let h = harness().await;
    let status = h.remote.apply(&AppSettings::default()).await;
    assert_eq!(status.state, RemoteListenerState::Disabled);
    assert!(status.listening_on.is_none());

    let status = h.remote.apply(&enabled_settings(0)).await;
    assert_eq!(status.state, RemoteListenerState::TokenMissing);
    assert!(!status.token_configured);
    assert!(status.listening_on.is_none());
}

#[tokio::test]
async fn remote_token_authenticates_only_the_remote_listener() {
    let h = harness().await;
    let (base, token) = h.start().await;
    assert_eq!(token.len(), 64, "256-bit random hex token");

    let state = format!("{base}/api/v1/state");
    assert_eq!(get_status(&h.http, state.clone(), Some(&token)).await, 200);
    assert_eq!(get_status(&h.http, state.clone(), None).await, 401);
    assert_eq!(get_status(&h.http, state.clone(), Some("wrong")).await, 401);
    // The local per-launch token is not a remote credential …
    assert_eq!(get_status(&h.http, state, Some(LOCAL_TOKEN)).await, 401);
    // … and the remote token is not a local one.
    let resp = h
        .local
        .clone()
        .oneshot(
            Request::get("/api/v1/state")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // Public discovery routes stay public.
    assert_eq!(
        get_status(&h.http, format!("{base}/api/v1/health"), None).await,
        200
    );
}

#[tokio::test]
async fn sensitive_endpoints_are_forbidden_to_remote_clients() {
    let h = harness().await;
    let (base, token) = h.start().await;
    let forbidden: &[(Method, &str, Option<Value>)] = &[
        (Method::GET, "/credentials", None),
        (
            Method::POST,
            "/credentials",
            Some(json!({"key": "GITHUB_TOKEN", "value": "x"})),
        ),
        (Method::DELETE, "/credentials/GITHUB_TOKEN", None),
        (Method::GET, "/workflow", None),
        (Method::PUT, "/workflow", Some(json!({"raw_markdown": "x"}))),
        (
            Method::POST,
            "/workflow/preview",
            Some(json!({"raw_markdown": "x"})),
        ),
        (Method::GET, "/settings", None),
        (
            Method::PUT,
            "/settings",
            Some(serde_json::to_value(AppSettings::default()).unwrap()),
        ),
        (Method::GET, "/diagnostics/export", None),
        (Method::GET, "/providers", None),
        (Method::GET, "/worktrees", None),
        (
            Method::POST,
            "/projects",
            Some(json!({"name": "x", "repo_path": "/tmp"})),
        ),
        (
            Method::POST,
            "/projects/validate",
            Some(json!({"repo_path": "/tmp"})),
        ),
        (Method::DELETE, "/projects/p1", None),
        (Method::PATCH, "/projects/p1", Some(json!({}))),
        (
            Method::PATCH,
            "/tasks/t1/status",
            Some(json!({"status": "done"})),
        ),
        (Method::GET, "/tasks/t1/diff", None),
        (Method::POST, "/prs/r1/approve-merge", None),
        (Method::GET, "/remote-access", None),
        (Method::POST, "/remote-access/token", None),
        (Method::DELETE, "/remote-access/token", None),
    ];
    for (method, path, body) in forbidden {
        let mut req = h
            .http
            .request(method.clone(), format!("{base}/api/v1{path}"))
            .bearer_auth(&token);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.unwrap();
        assert_eq!(
            resp.status(),
            403,
            "{method} {path} must be forbidden remotely"
        );
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["error"]["code"], "forbidden_scope", "{method} {path}");
    }
    // Without a token the answer is 401, not a scope hint.
    let resp = h
        .http
        .get(format!("{base}/api/v1/credentials"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    // The secret was not touched by any of the above.
    assert!(h
        .orchestrator
        .secret_store()
        .get_vault_secret("GITHUB_TOKEN")
        .is_none());
}

#[tokio::test]
async fn companion_operations_are_available_remotely() {
    let h = harness().await;
    let (base, token) = h.start().await;
    for path in [
        "/state",
        "/tasks",
        "/projects",
        "/approvals?only_pending=true",
        "/prs",
        "/events/recent",
    ] {
        assert_eq!(
            get_status(&h.http, format!("{base}/api/v1{path}"), Some(&token)).await,
            200,
            "GET {path}"
        );
    }
    // Task creation reaches the handler (unknown project → not a scope error).
    let resp = h
        .http
        .post(format!("{base}/api/v1/tasks"))
        .bearer_auth(&token)
        .json(&json!({"project_id": "missing", "title": "From the phone"}))
        .send()
        .await
        .unwrap();
    assert_ne!(resp.status(), 403);
    assert_ne!(resp.status(), 401);
    // Scheduler control is part of the companion scope.
    let resp = h
        .http
        .post(format!("{base}/api/v1/scheduler/pause"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn rotation_and_revocation_take_effect_immediately() {
    let h = harness().await;
    let (base, first) = h.start().await;
    let state = format!("{base}/api/v1/state");
    assert_eq!(get_status(&h.http, state.clone(), Some(&first)).await, 200);

    let second = h.remote.rotate_token().await.unwrap();
    assert_ne!(second.token, first);
    // Same listener (address unchanged), new credential.
    assert_eq!(
        second.status.listening_on.as_deref(),
        Some(base.trim_start_matches("http://"))
    );
    assert_eq!(get_status(&h.http, state.clone(), Some(&first)).await, 401);
    assert_eq!(
        get_status(&h.http, state.clone(), Some(&second.token)).await,
        200
    );

    let revoked = h.remote.revoke_token().await.unwrap();
    assert_eq!(revoked.state, RemoteListenerState::TokenMissing);
    assert!(!revoked.token_configured);
    assert!(h
        .orchestrator
        .secret_store()
        .get_vault_secret(REMOTE_TOKEN_KEY)
        .is_none());
    // The listener is gone.
    assert!(h.http.get(state).send().await.is_err());
}

#[tokio::test]
async fn open_event_streams_end_when_the_token_is_rotated() {
    let h = harness().await;
    let (base, token) = h.start().await;
    let mut resp = h
        .http
        .get(format!("{base}/api/v1/events"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let first = resp.chunk().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("event: ready"));

    h.remote.rotate_token().await.unwrap();
    // The stream finishes instead of staying authorized with the old token.
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(Some(_)) = resp.chunk().await {}
    })
    .await;
    assert!(ended.is_ok(), "event stream must end after rotation");

    // Reconnection with the new token works (mobile reconnect path).
    let new_token = h
        .orchestrator
        .secret_store()
        .get_vault_secret(REMOTE_TOKEN_KEY)
        .unwrap();
    let resp = h
        .http
        .get(format!("{base}/api/v1/events"))
        .bearer_auth(&new_token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn cors_allows_only_the_companion_and_configured_origins() {
    let h = harness().await;
    let mut settings = enabled_settings(0);
    settings.remote_allowed_origins = vec!["https://companion.example.com".into()];
    h.save_settings(&settings).await;
    let issued = h.remote.rotate_token().await.unwrap();
    let base = format!("http://{}", issued.status.listening_on.unwrap());

    let preflight = |origin: &'static str| {
        h.http
            .request(Method::OPTIONS, format!("{base}/api/v1/tasks"))
            .header("Origin", origin)
            .header("Access-Control-Request-Method", "POST")
            .header(
                "Access-Control-Request-Headers",
                "authorization,content-type",
            )
            .send()
    };
    for origin in ["tauri://localhost", "https://companion.example.com"] {
        let resp = preflight(origin).await.unwrap();
        assert_eq!(
            resp.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some(origin),
            "{origin} must be allowed"
        );
    }
    // Neither arbitrary sites nor the local desktop/dev origins are allowed remotely.
    for origin in ["https://evil.example", "http://localhost:1420", "null"] {
        let resp = preflight(origin).await.unwrap();
        assert!(
            resp.headers().get("access-control-allow-origin").is_none(),
            "{origin} must be rejected"
        );
    }
}

#[tokio::test]
async fn cleartext_forwarded_requests_are_refused() {
    let h = harness().await;
    let (base, token) = h.start().await;
    let resp = h
        .http
        .get(format!("{base}/api/v1/state"))
        .bearer_auth(&token)
        .header("X-Forwarded-Proto", "http")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "insecure_transport");
    let resp = h
        .http
        .get(format!("{base}/api/v1/state"))
        .bearer_auth(&token)
        .header("X-Forwarded-Proto", "https")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn listener_follows_configuration_changes() {
    let h = harness().await;
    let (base, token) = h.start().await;
    let old = format!("{base}/api/v1/state");

    // Move to another (ephemeral) port: a different spec rebinds.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let new_port = probe.local_addr().unwrap().port();
    drop(probe);
    let status = h.remote.apply(&enabled_settings(new_port)).await;
    assert_eq!(status.state, RemoteListenerState::Running);
    assert_eq!(status.listening_on, Some(format!("127.0.0.1:{new_port}")));
    assert!(h.http.get(old).bearer_auth(&token).send().await.is_err());
    assert_eq!(
        get_status(
            &h.http,
            format!("http://127.0.0.1:{new_port}/api/v1/state"),
            Some(&token)
        )
        .await,
        200
    );

    // Disable: stopped.
    let status = h.remote.apply(&AppSettings::default()).await;
    assert_eq!(status.state, RemoteListenerState::Disabled);
    assert!(h
        .http
        .get(format!("http://127.0.0.1:{new_port}/api/v1/health"))
        .send()
        .await
        .is_err());
}

#[tokio::test]
async fn bind_failures_and_unsafe_addresses_are_reported() {
    let h = harness().await;
    h.remote.rotate_token().await.unwrap();

    // Port already in use.
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = blocker.local_addr().unwrap().port();
    let status = h.remote.apply(&enabled_settings(busy)).await;
    assert_eq!(status.state, RemoteListenerState::Failed);
    assert!(status.error.unwrap().contains(&busy.to_string()));
    drop(blocker);
    let status = h.remote.apply(&enabled_settings(busy)).await;
    assert_eq!(
        status.state,
        RemoteListenerState::Running,
        "recovers once the port is free"
    );

    // A non-loopback bind address is never used.
    let mut exposed = enabled_settings(0);
    exposed.remote_bind_address = "0.0.0.0".into();
    let status = h.remote.apply(&exposed).await;
    assert_eq!(status.state, RemoteListenerState::Failed);
    assert!(status.listening_on.is_none());

    // The local API's own port is refused.
    h.remote.set_local_port(busy);
    let status = h.remote.apply(&enabled_settings(busy)).await;
    assert_eq!(status.state, RemoteListenerState::Failed);
}

#[tokio::test]
async fn local_administration_endpoints_manage_remote_access() {
    let h = harness().await;
    let (status, body) = h
        .local_request(Method::GET, "/api/v1/remote-access", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["token_configured"], json!(false));
    assert_eq!(body["state"], json!("disabled"));

    // Enabling through the settings API applies immediately (no restart).
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut settings = enabled_settings(port);
    let (status, _) = h
        .local_request(
            Method::PUT,
            "/api/v1/settings",
            Some(serde_json::to_value(&settings).unwrap()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = h
        .local_request(Method::GET, "/api/v1/remote-access", None)
        .await;
    assert_eq!(body["state"], json!("token_missing"));

    // Issue a token: returned once, never listed.
    let resp = h
        .local
        .clone()
        .oneshot(
            Request::post("/api/v1/remote-access/token")
                .header(header::AUTHORIZATION, format!("Bearer {LOCAL_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
    let issued: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let token = issued["token"].as_str().unwrap().to_string();
    assert_eq!(issued["status"]["state"], json!("running"));

    let (_, body) = h
        .local_request(Method::GET, "/api/v1/remote-access", None)
        .await;
    assert!(
        !body.to_string().contains(&token),
        "status never echoes the token"
    );
    let (_, body) = h
        .local_request(Method::GET, "/api/v1/credentials", None)
        .await;
    assert!(!body.to_string().contains(&token));
    assert!(!body["configured"]
        .as_array()
        .unwrap()
        .contains(&json!(REMOTE_TOKEN_KEY)));
    // The generic credential API cannot overwrite the remote token.
    let (status, _) = h
        .local_request(
            Method::POST,
            "/api/v1/credentials",
            Some(json!({"key": REMOTE_TOKEN_KEY, "value": "attacker-chosen-token-value-123456"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Unsafe settings are rejected before they are stored.
    settings.remote_bind_address = "0.0.0.0".into();
    let (status, body) = h
        .local_request(
            Method::PUT,
            "/api/v1/settings",
            Some(serde_json::to_value(&settings).unwrap()),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    settings.remote_bind_address = "127.0.0.1".into();
    settings.remote_allowed_origins = vec!["*".into()];
    let (status, _) = h
        .local_request(
            Method::PUT,
            "/api/v1/settings",
            Some(serde_json::to_value(&settings).unwrap()),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Revoke.
    let (status, body) = h
        .local_request(Method::DELETE, "/api/v1/remote-access/token", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], json!("token_missing"));
}

#[tokio::test]
async fn remote_access_endpoints_report_unavailable_without_a_manager() {
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = Arc::new(
        SymphonyOrchestrator::new(
            Storage::connect_in_memory().await.unwrap(),
            None,
            SecretStore::new(dir.path()).unwrap(),
        )
        .await
        .unwrap(),
    );
    let app = build_router(
        orchestrator,
        &ServerConfig::local(0, Some(LOCAL_TOKEN.into())),
    );
    let resp = app
        .oneshot(
            Request::get("/api/v1/remote-access")
                .header(header::AUTHORIZATION, format!("Bearer {LOCAL_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
}
