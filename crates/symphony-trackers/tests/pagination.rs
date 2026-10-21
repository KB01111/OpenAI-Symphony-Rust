//! Issue listings follow the trackers' pagination instead of stopping at the first page, and stop
//! at a bounded number of pages.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use symphony_trackers::{GitHubTracker, LinearTracker, TrackerAdapter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

type Handler = dyn Fn(&str, &str) -> Value + Send + Sync;

/// A one-request-per-connection HTTP server answering every request with `handler(target, body)`
/// as JSON. Returns its base URL and the requests it received (`target` and body).
async fn serve(handler: Box<Handler>) -> (String, Arc<Mutex<Vec<(String, String)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let handler: Arc<Handler> = handler.into();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let handler = handler.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let (head_len, content_length) = loop {
                    let n = stream.read(&mut buf).await.unwrap();
                    assert!(n > 0, "connection closed mid-request");
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..pos]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .map(|v| v.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        break (pos + 4, length);
                    }
                };
                while raw.len() < head_len + content_length {
                    let n = stream.read(&mut buf).await.unwrap();
                    raw.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&raw[..head_len]).to_string();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let body = String::from_utf8_lossy(&raw[head_len..]).to_string();
                let response = handler(&target, &body).to_string();
                seen.lock().unwrap().push((target, body));
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
                stream.write_all(reply.as_bytes()).await.unwrap();
                let _ = stream.shutdown().await;
            });
        }
    });
    (base, requests)
}

fn query_param(target: &str, name: &str) -> Option<usize> {
    target
        .split_once('?')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
        .and_then(|v| v.parse().ok())
}

fn github_issues(first: usize, count: usize) -> Value {
    Value::Array(
        (first..first + count)
            .map(|n| json!({ "number": n, "title": format!("Issue {n}"), "state": "open", "labels": [] }))
            .collect(),
    )
}

#[tokio::test]
async fn github_listing_reads_every_page() {
    // 250 open issues: two full pages of 100 and a short last page.
    let (base, requests) = serve(Box::new(|target, _| {
        let page = query_param(target, "page").unwrap_or(1);
        let first = (page - 1) * 100 + 1;
        github_issues(first, 250usize.saturating_sub(first - 1).min(100))
    }))
    .await;
    let tracker = GitHubTracker::new(Some(base), "owner/repo".into(), None, None);

    let issues = tracker
        .fetch_issues_by_states(&["open".into()])
        .await
        .unwrap();

    assert_eq!(issues.len(), 250);
    assert_eq!(issues.last().unwrap().identifier, "GH-250");
    let pages: Vec<_> = requests
        .lock()
        .unwrap()
        .iter()
        .map(|(target, _)| query_param(target, "page"))
        .collect();
    assert_eq!(pages, vec![Some(1), Some(2), Some(3)]);
}

#[tokio::test]
async fn github_listing_stops_at_the_page_limit() {
    let (base, requests) = serve(Box::new(|target, _| {
        let page = query_param(target, "page").unwrap_or(1);
        github_issues((page - 1) * 100 + 1, 100)
    }))
    .await;
    let tracker = GitHubTracker::new(Some(base), "owner/repo".into(), None, None);

    let issues = tracker
        .fetch_issues_by_states(&["open".into()])
        .await
        .unwrap();

    assert_eq!(issues.len(), 1000);
    assert_eq!(requests.lock().unwrap().len(), 10);
}

#[tokio::test]
async fn linear_listing_follows_the_cursor() {
    let (base, requests) = serve(Box::new(|_, body| {
        let request: Value = serde_json::from_str(body).unwrap();
        let node = |id: &str| {
            json!({ "id": id, "identifier": id.to_uppercase(), "title": id, "state": { "name": "Todo" } })
        };
        if request.pointer("/variables/after") == Some(&json!("cursor-1")) {
            json!({ "data": { "issues": {
                "nodes": [node("sym-3")],
                "pageInfo": { "hasNextPage": false, "endCursor": "cursor-2" }
            } } })
        } else {
            json!({ "data": { "issues": {
                "nodes": [node("sym-1"), node("sym-2")],
                "pageInfo": { "hasNextPage": true, "endCursor": "cursor-1" }
            } } })
        }
    }))
    .await;
    let tracker = LinearTracker::new(
        Some(base),
        "project".into(),
        Some("lin_api_test".into()),
        Vec::new(),
    );

    let issues = tracker
        .fetch_issues_by_states(&["Todo".into()])
        .await
        .unwrap();

    let identifiers: Vec<_> = issues.iter().map(|i| i.identifier.as_str()).collect();
    assert_eq!(identifiers, vec!["SYM-1", "SYM-2", "SYM-3"]);
    let afters: Vec<Value> = requests
        .lock()
        .unwrap()
        .iter()
        .map(|(_, body)| {
            let request: Value = serde_json::from_str(body).unwrap();
            request.pointer("/variables/after").cloned().unwrap()
        })
        .collect();
    assert_eq!(afters, vec![Value::Null, json!("cursor-1")]);
}

#[tokio::test]
async fn linear_listing_rejects_a_cursor_that_does_not_advance() {
    let (base, requests) = serve(Box::new(|_, _| {
        json!({ "data": { "issues": {
            "nodes": [{ "id": "sym-1", "identifier": "SYM-1", "title": "t", "state": { "name": "Todo" } }],
            "pageInfo": { "hasNextPage": true, "endCursor": "stuck" }
        } } })
    }))
    .await;
    let tracker = LinearTracker::new(
        Some(base),
        "project".into(),
        Some("lin_api_test".into()),
        Vec::new(),
    );

    let err = tracker
        .fetch_issues_by_states(&["Todo".into()])
        .await
        .unwrap_err();

    assert!(err.to_string().contains("endCursor"), "{err}");
    assert_eq!(requests.lock().unwrap().len(), 2);
}
