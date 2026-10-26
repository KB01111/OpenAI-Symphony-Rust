//! The host-side `github_api` / `linear_graphql` tools act with Symphony's own credentials on
//! behalf of an agent that reads untrusted text. Requests outside the policy must never leave the
//! host; allowed ones must still work.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use symphony_trackers::{GitHubTracker, LinearTracker};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Records `METHOD target` + authorization of every request and answers `{}`.
async fn recording_server() -> (String, Arc<Mutex<Vec<String>>>) {
    recording_server_redirecting(None).await
}

/// Like [`recording_server`], answering `302` to `redirect` when given.
async fn recording_server_redirecting(
    redirect: Option<String>,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let log = log.clone();
            let redirect = redirect.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&raw).to_string();
                let mut first = head.lines().next().unwrap_or("").split_whitespace();
                let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
                let auth = head
                    .lines()
                    .find(|l| l.to_lowercase().starts_with("authorization:"))
                    .map(|l| l.split_once(':').unwrap().1.trim().to_string())
                    .unwrap_or_default();
                log.lock()
                    .unwrap()
                    .push(format!("{method} {target} [{auth}]"));
                let body = "{}";
                let reply = match &redirect {
                    Some(location) => format!(
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    ),
                    None => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                };
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (base, seen)
}

fn message(output: &str) -> String {
    serde_json::from_str::<Value>(output).unwrap()["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn github_requests_outside_the_policy_never_leave_the_host() {
    let (base, seen) = recording_server().await;
    let tracker = GitHubTracker::new(
        Some(base),
        "acme/widgets".into(),
        Some("ghp_hosttoken_hosttoken_hosttoken".into()),
        None,
    );

    for (method, path) in [
        ("PUT", "/repos/acme/widgets/pulls/7/merge"),
        ("DELETE", "/repos/acme/widgets"),
        ("GET", "/repos/acme/secret-project/contents/.env"),
        ("GET", "/user"),
    ] {
        let (ok, output) = tracker
            .execute_github_dynamic_tool(json!({ "method": method, "path": path }))
            .await;
        assert!(!ok, "{method} {path} must be refused");
        assert!(!message(&output).is_empty(), "{output}");
    }
    assert!(
        seen.lock().unwrap().is_empty(),
        "refused calls reached GitHub: {:?}",
        seen.lock().unwrap()
    );

    let (ok, _) = tracker
        .execute_github_dynamic_tool(json!({
            "method": "POST",
            "path": "/repos/acme/widgets/issues/5/comments",
            "body": { "body": "Implemented" }
        }))
        .await;
    assert!(ok);
    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(requests[0].starts_with("POST /repos/acme/widgets/issues/5/comments"));
    // The host adds its credential; the agent never supplied one.
    assert!(requests[0].contains("ghp_hosttoken"), "{requests:?}");
}

#[tokio::test]
async fn linear_mutations_outside_the_allowlist_never_leave_the_host() {
    let (base, seen) = recording_server().await;
    let tracker = LinearTracker::new(
        Some(base),
        "proj".into(),
        Some("lin_api_hosttoken_hosttoken".into()),
        Vec::new(),
    );

    let (ok, output) = tracker
        .execute_linear_dynamic_tool(
            json!({ "query": "mutation { issueDelete(id: \"x\") { success } }" }),
        )
        .await;
    assert!(!ok);
    assert!(message(&output).contains("issueDelete"), "{output}");
    assert!(seen.lock().unwrap().is_empty());

    let (ok, _) = tracker
        .execute_linear_dynamic_tool(json!({ "query": "query { viewer { id } }" }))
        .await;
    // Reaches the (mock) API; the empty answer is reported as a GraphQL payload, not a policy error.
    let _ = ok;
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_redirect_never_carries_the_host_credentials_to_another_host() {
    // A permitted repository GET whose answer redirects elsewhere (here: another origin).
    let (elsewhere, elsewhere_seen) = recording_server().await;
    let (api, api_seen) = recording_server_redirecting(Some(format!("{elsewhere}/steal"))).await;
    let tracker = GitHubTracker::new(
        Some(api),
        "acme/widgets".into(),
        Some("ghp_hosttoken_hosttoken_hosttoken".into()),
        None,
    );
    let (ok, _) = tracker
        .execute_github_dynamic_tool(
            json!({ "method": "GET", "path": "/repos/acme/widgets/issues/1" }),
        )
        .await;
    assert!(!ok, "a redirect is not a successful answer");
    assert_eq!(api_seen.lock().unwrap().len(), 1);
    assert!(
        elsewhere_seen.lock().unwrap().is_empty(),
        "the redirect target was contacted: {:?}",
        elsewhere_seen.lock().unwrap()
    );
}

/// A Linear stand-in that answers the scope lookups (`issue(id)` / `comment(id)`) from a table and
/// records every other operation it receives.
async fn linear_with_projects() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let log = log.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 8192];
                let body = loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if body.len() >= length {
                            break body.to_string();
                        }
                    }
                };
                let request: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                let query = request["query"].as_str().unwrap_or_default();
                let id = request["variables"]["id"].as_str().unwrap_or_default();
                let project = |slug: &str| json!({ "slugId": slug });
                let answer = if query.contains("SymphonyScopeIssue") {
                    match id {
                        "issue-in" => {
                            json!({ "data": { "issue": { "project": project("proj") } } })
                        }
                        "issue-out" => {
                            json!({ "data": { "issue": { "project": project("elsewhere") } } })
                        }
                        _ => json!({ "errors": [{ "message": "Entity not found" }] }),
                    }
                } else if query.contains("SymphonyScopeComment") {
                    match id {
                        "comment-in" => {
                            json!({ "data": { "comment": { "issue": { "project": project("proj") } } } })
                        }
                        _ => json!({ "errors": [{ "message": "Entity not found" }] }),
                    }
                } else {
                    log.lock().unwrap().push(query.to_string());
                    json!({ "data": { "ok": true } })
                };
                let text = answer.to_string();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    (base, seen)
}

#[tokio::test]
async fn linear_mutations_change_only_issues_of_the_configured_project() {
    let (base, sent) = linear_with_projects().await;
    let tracker = LinearTracker::new(
        Some(base),
        "proj".into(),
        Some("lin_api_hosttoken_hosttoken".into()),
        Vec::new(),
    );
    let comment = "mutation C($i: String!) { commentCreate(input: { issueId: $i, body: \"done\" }) { success } }";

    let (ok, output) = tracker
        .execute_linear_dynamic_tool(json!({ "query": comment, "variables": { "i": "issue-in" } }))
        .await;
    assert!(ok, "{output}");
    assert_eq!(
        sent.lock().unwrap().len(),
        1,
        "the mutation on the project's issue is sent"
    );

    for (query, variables) in [
        (comment, json!({ "i": "issue-out" })),
        (comment, json!({ "i": "unknown" })),
        ("mutation { commentUpdate(id: \"comment-elsewhere\", input: { body: \"x\" }) { success } }", json!({})),
        // One target in the project does not let a second one through.
        ("mutation { a: issueUpdate(id: \"issue-in\", input: {}) { success } b: issueUpdate(id: \"issue-out\", input: {}) { success } }", json!({})),
    ] {
        let (ok, output) = tracker.execute_linear_dynamic_tool(json!({ "query": query, "variables": variables })).await;
        assert!(!ok, "must be refused: {query}");
        assert!(message(&output).contains("configured project"), "{output}");
    }
    assert_eq!(
        sent.lock().unwrap().len(),
        1,
        "refused mutations never reach Linear"
    );

    let (ok, output) = tracker
        .execute_linear_dynamic_tool(json!({ "query": "mutation { commentUpdate(id: \"comment-in\", input: { body: \"x\" }) { success } }" }))
        .await;
    assert!(ok, "{output}");
    // Reads stay possible (they are not confined to the project).
    let (ok, _) = tracker
        .execute_linear_dynamic_tool(json!({ "query": "query { viewer { id } }" }))
        .await;
    assert!(ok);
    assert_eq!(sent.lock().unwrap().len(), 3);
}
