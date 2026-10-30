//! Router-level tests: route registration (axum 0.8 path syntax), auth, CORS and error mapping.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use symphony_orchestrator::SymphonyOrchestrator;
use symphony_platform::SecretStore;
use symphony_server::{build_router, ServerConfig, API_VERSION};
use symphony_storage::Storage;
use tower::ServiceExt;

const TOKEN: &str = "test-token-0123456789abcdef";

async fn router() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::connect_in_memory().await.unwrap();
    let secrets = SecretStore::new(dir.path()).unwrap();
    let orch = Arc::new(
        SymphonyOrchestrator::new(storage, None, secrets)
            .await
            .unwrap(),
    );
    let config = ServerConfig {
        default_workspace_root: Some(dir.path().join("workspaces")),
        ..ServerConfig::local(0, Some(TOKEN.to_string()))
    };
    (build_router(orch, &config), dir)
}

fn authed(method: Method, uri: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    match body {
        Some(b) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn json_body(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn public_routes_do_not_require_token() {
    let (app, _dir) = router().await;
    let resp = app
        .clone()
        .oneshot(Request::get("/api/v1/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = app
        .oneshot(Request::get("/api/v1/runtime").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["auth_required"], json!(true));
    assert_eq!(body["api_version"], json!(API_VERSION));
}

#[tokio::test]
async fn protected_routes_reject_missing_or_wrong_token() {
    let (app, _dir) = router().await;
    for uri in [
        "/api/v1/state",
        "/api/v1/tasks",
        "/api/v1/settings",
        "/api/v1/SYM-1",
    ] {
        let resp = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
    let resp = app
        .oneshot(
            Request::get("/api/v1/state")
                .header(header::AUTHORIZATION, "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json_body(resp).await["error"]["code"],
        json!("unauthorized")
    );
}

#[tokio::test]
async fn cors_allows_only_listed_origins() {
    let (app, _dir) = router().await;
    let preflight = |origin: &str| {
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/api/v1/state")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
            .body(Body::empty())
            .unwrap()
    };
    let ok = app
        .clone()
        .oneshot(preflight("http://tauri.localhost"))
        .await
        .unwrap();
    assert_eq!(
        ok.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "http://tauri.localhost"
    );
    let evil = app
        .oneshot(preflight("https://evil.example"))
        .await
        .unwrap();
    assert!(evil
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
}

#[tokio::test]
async fn invalid_requests_map_to_client_errors() {
    let (app, _dir) = router().await;
    // Unknown task -> 404
    let resp = app
        .clone()
        .oneshot(authed(Method::GET, "/api/v1/tasks/does-not-exist", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        json_body(resp).await["error"]["code"],
        json!("task_not_found")
    );

    // Non-repository path -> 400 invalid_repository
    let resp = app
        .clone()
        .oneshot(authed(
            Method::POST,
            "/api/v1/projects",
            Some(json!({ "name": "x", "repo_path": "/definitely/not/here" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(resp).await["error"]["code"],
        json!("invalid_repository")
    );

    // Credentials outside the allowlist are refused (no environment injection).
    let resp = app
        .clone()
        .oneshot(authed(
            Method::POST,
            "/api/v1/credentials",
            Some(json!({ "key": "PATH", "value": "/tmp/evil" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Invalid workflow preview reports valid=false with 200 for inline editor diagnostics.
    let resp = app
        .clone()
        .oneshot(authed(
            Method::POST,
            "/api/v1/workflow/preview",
            Some(json!({ "raw_markdown": "---\n- a\n- b\n---\nbody" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["valid"], json!(false));
    assert_eq!(body["error_code"], json!("workflow_front_matter_not_a_map"));

    // Invalid settings are rejected.
    let mut settings = json_body(
        app.clone()
            .oneshot(authed(Method::GET, "/api/v1/settings", None))
            .await
            .unwrap(),
    )
    .await;
    settings["theme"] = json!("neon");
    let resp = app
        .oneshot(authed(Method::PUT, "/api/v1/settings", Some(settings)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_client_that_omits_the_retention_setting_keeps_the_stored_value() {
    let (app, _dir) = router().await;
    let get = |app: axum::Router| async move {
        json_body(
            app.oneshot(authed(Method::GET, "/api/v1/settings", None))
                .await
                .unwrap(),
        )
        .await
    };
    let put = |app: axum::Router, body: serde_json::Value| async move {
        app.oneshot(authed(Method::PUT, "/api/v1/settings", Some(body)))
            .await
            .unwrap()
    };

    let mut settings = get(app.clone()).await;
    assert_eq!(settings["event_retention_days"], json!(30));

    // A current client saves its own value.
    settings["event_retention_days"] = json!(0);
    assert_eq!(
        put(app.clone(), settings.clone()).await.status(),
        StatusCode::OK
    );
    assert_eq!(get(app.clone()).await["event_retention_days"], json!(0));

    // An older client that does not know the field must not reset it to the default.
    settings["theme"] = json!("dark");
    settings
        .as_object_mut()
        .unwrap()
        .remove("event_retention_days");
    let resp = put(app.clone(), settings.clone()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json_body(resp).await["event_retention_days"], json!(0));
    let after = get(app.clone()).await;
    assert_eq!(after["event_retention_days"], json!(0));
    assert_eq!(after["theme"], json!("dark"));

    // Out-of-range values are rejected.
    settings["event_retention_days"] = json!(3651);
    assert_eq!(put(app, settings).await.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workflow_config_never_leaks_tracker_tokens() {
    let (app, _dir) = router().await;
    let raw = "---\ntracker:\n  kind: github\n  provider:\n    repo: a/b\n    token: ghp_supersecretvalue123456\n---\nHi {{ issue.identifier }}";
    let resp = app
        .oneshot(authed(
            Method::POST,
            "/api/v1/workflow/preview",
            Some(json!({ "raw_markdown": raw })),
        ))
        .await
        .unwrap();
    let body = json_body(resp).await;
    assert_eq!(body["valid"], json!(true));
    assert_eq!(
        body["config"]["tracker_provider"]["token"],
        json!("[redacted]")
    );
    assert!(!body.to_string().contains("ghp_supersecretvalue123456"));
}

#[tokio::test]
async fn registering_a_subdirectory_stores_the_checkout_root() {
    let (app, dir) = router().await;
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("src/nested")).unwrap();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@symphony.local"]);
    git(&["config", "user.name", "T"]);
    std::fs::write(repo.join("README.md"), "x\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);

    let nested = repo.join("src/nested");
    let resp = app
        .clone()
        .oneshot(authed(
            Method::POST,
            "/api/v1/projects",
            Some(json!({ "name": "nested", "repo_path": nested.to_string_lossy() })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let project = json_body(resp).await;
    let stored = std::path::PathBuf::from(project["repo_path"].as_str().unwrap());
    assert_eq!(
        stored.canonicalize().unwrap(),
        repo.canonicalize().unwrap(),
        "worktrees must be created from the checkout root"
    );
}

#[tokio::test]
async fn backups_are_created_and_listed_through_the_local_api() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::connect(dir.path().join("symphony.sqlite"))
        .await
        .unwrap();
    let secrets = SecretStore::new(dir.path()).unwrap();
    let orch = Arc::new(
        SymphonyOrchestrator::new(storage, None, secrets)
            .await
            .unwrap(),
    );
    let app = build_router(
        orch,
        &ServerConfig {
            backup_root: Some(dir.path().join("backups")),
            ..ServerConfig::local(0, Some(TOKEN.to_string()))
        },
    );

    // Authentication is required.
    let anonymous = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/backups")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let empty = app
        .clone()
        .oneshot(authed(Method::GET, "/api/v1/backups", None))
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::OK);
    assert_eq!(json_body(empty).await["backups"], json!([]));

    let created = app
        .clone()
        .oneshot(authed(Method::POST, "/api/v1/backups", None))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    assert!(created["id"]
        .as_str()
        .unwrap()
        .starts_with("symphony-backup-"));
    assert!(created["size_bytes"].as_u64().unwrap() > 0);
    assert!(std::path::Path::new(created["path"].as_str().unwrap())
        .join("symphony.sqlite")
        .is_file());

    let listed = json_body(
        app.oneshot(authed(Method::GET, "/api/v1/backups", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(listed["backups"].as_array().unwrap().len(), 1);
    assert_eq!(listed["backups"][0]["id"], created["id"]);
}

#[tokio::test]
async fn backups_are_unavailable_on_a_host_that_does_not_manage_them() {
    let (app, _dir) = router().await;
    let resp = app
        .oneshot(authed(Method::POST, "/api/v1/backups", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        json_body(resp).await["error"]["code"],
        "backups_unavailable"
    );
}

#[tokio::test]
async fn creating_a_pull_request_needs_a_token_and_an_existing_review() {
    let (app, _dir) = router().await;
    let anonymous = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/prs/pr-1/create-pull-request")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let missing = app
        .oneshot(authed(
            Method::POST,
            "/api/v1/prs/does-not-exist/create-pull-request",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn task_events_page_backwards_from_a_known_event_only() {
    let (app, _dir) = router().await;
    let ok = app
        .clone()
        .oneshot(authed(Method::GET, "/api/v1/tasks/t1/events?limit=5", None))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(json_body(ok).await, json!([]));

    // A cursor that is not an event of the task is an error, never an empty "beginning of history".
    let resp = app
        .clone()
        .oneshot(authed(
            Method::GET,
            "/api/v1/tasks/t1/events?limit=5&before=unknown-event",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(resp).await["error"]["code"], json!("not_found"));
}
