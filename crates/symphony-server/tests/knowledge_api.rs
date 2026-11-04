//! The Knowledge Base endpoints through the real router: auth, validation, git-backed data and
//! the local-only scope.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use symphony_orchestrator::SymphonyOrchestrator;
use symphony_platform::SecretStore;
use symphony_server::{build_router, ServerConfig};
use symphony_storage::Storage;
use tower::ServiceExt;

const TOKEN: &str = "test-token-0123456789abcdef";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Fixture {
    app: axum::Router,
    _dir: tempfile::TempDir,
    project_id: String,
    repo: std::path::PathBuf,
}

async fn fixture(enable_knowledge: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "first"]);

    let storage = Storage::connect_in_memory().await.unwrap();
    let secrets = SecretStore::new(dir.path()).unwrap();
    let orch = Arc::new(
        SymphonyOrchestrator::new(storage, None, secrets)
            .await
            .unwrap(),
    );
    if enable_knowledge {
        orch.enable_knowledge(&dir.path().join("data"))
            .await
            .unwrap();
    }
    let config = ServerConfig {
        default_workspace_root: Some(dir.path().join("workspaces")),
        ..ServerConfig::local(0, Some(TOKEN.to_string()))
    };
    let app = build_router(orch, &config);
    let resp = app
        .clone()
        .oneshot(authed(
            Method::POST,
            "/api/v1/projects",
            Some(json!({ "name": "demo", "repo_path": repo.to_string_lossy(), "default_branch": "main" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let project_id = json_body(resp).await["id"].as_str().unwrap().to_string();
    Fixture {
        app,
        _dir: dir,
        project_id,
        repo,
    }
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

async fn get(f: &Fixture, path: &str) -> (StatusCode, Value) {
    let resp = f
        .app
        .clone()
        .oneshot(authed(
            Method::GET,
            &format!("/api/v1/projects/{}/knowledge{path}", f.project_id),
            None,
        ))
        .await
        .unwrap();
    let status = resp.status();
    (status, json_body(resp).await)
}

#[tokio::test]
async fn knowledge_routes_require_the_token() {
    let f = fixture(true).await;
    for uri in [
        format!("/api/v1/projects/{}/knowledge", f.project_id),
        format!("/api/v1/projects/{}/knowledge/commits", f.project_id),
        "/api/v1/knowledge/engines".to_string(),
    ] {
        let resp = f
            .app
            .clone()
            .oneshot(Request::get(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn a_runtime_without_the_knowledge_base_says_so() {
    let f = fixture(false).await;
    let (status, body) = get(&f, "").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not enabled"));
}

#[tokio::test]
async fn worktrees_and_the_timeline_come_from_the_repository() {
    let f = fixture(true).await;
    // A second worktree on a new branch with its own commit.
    let wt = f.repo.parent().unwrap().join("wt");
    git(
        &f.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
        ],
    );
    std::fs::write(wt.join("b.txt"), "x\n").unwrap();
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "-q", "-m", "feature work"]);
    let feature_sha = git(&wt, &["rev-parse", "HEAD"]);
    let first_sha = git(&f.repo, &["rev-parse", "main"]);

    let (status, worktrees) = get(&f, "/worktrees").await;
    assert_eq!(status, StatusCode::OK);
    let list = worktrees.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list
        .iter()
        .any(|w| w["kind"] == json!("main") && w["branch"] == json!("main")));
    let feature = list
        .iter()
        .find(|w| w["branch"] == json!("feature"))
        .unwrap();
    assert_eq!(feature["head_sha"], json!(feature_sha));
    assert_eq!(feature["ahead_of_default"], json!(1));
    assert_eq!(
        feature["kind"],
        json!("external"),
        "no Symphony task owns it"
    );

    let (status, timeline) = get(&f, "/commits?limit=50").await;
    assert_eq!(status, StatusCode::OK);
    let commits = timeline["commits"].as_array().unwrap();
    assert_eq!(
        commits
            .iter()
            .filter(|c| c["sha"] == json!(first_sha))
            .count(),
        1
    );
    let shared = commits
        .iter()
        .find(|c| c["sha"] == json!(first_sha))
        .unwrap();
    assert_eq!(
        shared["worktrees"].as_array().unwrap().len(),
        2,
        "both worktrees contain the first commit"
    );
    let own = commits
        .iter()
        .find(|c| c["sha"] == json!(feature_sha))
        .unwrap();
    assert_eq!(own["worktrees"].as_array().unwrap().len(), 1);
    assert_eq!(own["on_default_branch"], json!(false));

    let (status, detail) = get(&f, &format!("/commits/{feature_sha}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["files"][0]["path"], json!("b.txt"));
    assert_eq!(detail["files"][0]["status"], json!("added"));
    assert_eq!(detail["files"][0]["hunks"][0]["new_lines"], json!(1));
    // Short ids resolve to the full one.
    let (status, short) = get(&f, &format!("/commits/{}", &feature_sha[..10])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(short["commit"]["sha"], json!(feature_sha));

    let (status, diff) = get(&f, &format!("/commits/{feature_sha}/diff")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(diff["diff"].as_str().unwrap().contains("+x"));
}

#[tokio::test]
async fn commit_ids_are_validated_and_unknown_ones_are_not_found() {
    let f = fixture(true).await;
    for bad in ["HEAD", "main", "--upload-pack=x", "ABCDEF1234", "zzzzzzz"] {
        let (status, _) = get(&f, &format!("/commits/{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (status, _) = get(&f, &format!("/commits/{}", "0".repeat(40))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn graph_views_need_an_index_and_say_what_is_missing() {
    let f = fixture(true).await;
    let (status, body) = get(&f, "/graph").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not been indexed"));
    let (status, _) = get(&f, "/search?q=anything").await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = get(&f, "/node/references?node_id=anything").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "precise references need an index too"
    );
    let (status, _) = get(&f, "/search?q=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Without an index the overview still works and reports it.
    let (status, overview) = get(&f, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(overview["index"], Value::Null);
    assert_eq!(overview["worktrees_total"], json!(1));
    assert!(overview["engines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["id"] == json!("git") && e["available"] == json!(true)));
    let codes: Vec<_> = overview["health"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["code"].as_str().unwrap())
        .collect();
    assert!(
        codes.contains(&"engine_missing") || codes.contains(&"not_indexed"),
        "{codes:?}"
    );

    // Impact without an engine or index still lists the files and says why symbols are missing.
    let sha = git(&f.repo, &["rev-parse", "HEAD"]);
    let (status, impact) = get(&f, &format!("/commits/{sha}/impact")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(impact["files"][0]["path"], json!("a.txt"));
    assert!(impact["unavailable_reason"].is_string());

    let (status, analysis) = get(&f, &format!("/commits/{sha}/analysis")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(analysis["analysis"], Value::Null);
}

#[tokio::test]
async fn chat_validates_questions_and_starts_empty() {
    let f = fixture(true).await;
    let (status, messages) = get(&f, "/chat").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(messages, json!([]));
    for question in ["", "   "] {
        let resp = f
            .app
            .clone()
            .oneshot(authed(
                Method::POST,
                &format!("/api/v1/projects/{}/knowledge/chat", f.project_id),
                Some(json!({ "question": question })),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{question:?}");
    }
    let resp = f
        .app
        .clone()
        .oneshot(authed(
            Method::DELETE,
            &format!("/api/v1/projects/{}/knowledge/chat", f.project_id),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn unknown_projects_are_not_found() {
    let f = fixture(true).await;
    let resp = f
        .app
        .clone()
        .oneshot(authed(Method::GET, "/api/v1/projects/nope/knowledge", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[test]
fn no_knowledge_operation_is_available_on_the_remote_listener() {
    let doc = serde_json::to_value(symphony_server::openapi()).unwrap();
    let mut seen = 0;
    for (path, item) in doc["paths"].as_object().unwrap() {
        if !path.contains("/knowledge") {
            continue;
        }
        for (_method, operation) in item.as_object().unwrap() {
            assert_eq!(
                operation["x-symphony-remote"],
                json!(false),
                "{path} must stay local"
            );
            assert_eq!(operation["tags"], json!(["knowledge"]));
            seen += 1;
        }
    }
    assert!(
        seen >= 16,
        "expected the Knowledge Base operations, saw {seen}"
    );
}
