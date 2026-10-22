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
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
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
