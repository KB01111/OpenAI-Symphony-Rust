//! The connection test against a stand-in model service on loopback.

use symphony_agents::connection_probe::{probe, probe_models_url};
use symphony_core::ProfileStatus;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serves one canned response and returns the request head it received.
async fn serve(
    status: &str,
    extra_headers: &str,
    body: &str,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/models", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\n{extra_headers}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // Read until the end of the request head: one read may return only part of it.
        let mut head = Vec::new();
        let mut buf = vec![0u8; 8192];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
        }
        // The client may stop reading an oversized body early.
        let _ = stream.write_all(response.as_bytes()).await;
        String::from_utf8_lossy(&head).to_string()
    });
    (url, task)
}

#[tokio::test]
async fn a_key_the_service_accepts_is_verified_and_the_models_are_listed() {
    let (url, request) = serve(
        "200 OK",
        "",
        r#"{"data":[{"id":"openai/gpt-5.5"},{"id":"anthropic/claude-sonnet-5.5"}]}"#,
    )
    .await;
    let result = probe_models_url(&url, "sk-test-key").await;
    assert_eq!(result.status, ProfileStatus::Verified, "{result:?}");
    assert_eq!(
        result.models,
        ["anthropic/claude-sonnet-5.5", "openai/gpt-5.5"]
    );
    assert_eq!(result.detail, "2 models");
    let head = request.await.unwrap().to_lowercase();
    assert!(head.starts_with("get /models "), "{head}");
    assert!(head.contains("authorization: bearer sk-test-key"), "{head}");
}

#[tokio::test]
async fn a_refused_key_is_an_error_that_does_not_echo_the_key_or_the_body() {
    let (url, _) = serve("401 Unauthorized", "", r#"{"error":"bad key sk-test-key"}"#).await;
    let result = probe_models_url(&url, "sk-test-key").await;
    assert_eq!(result.status, ProfileStatus::Error);
    assert!(result.detail.contains("refused the key"), "{result:?}");
    assert!(!result.detail.contains("sk-test-key"));
    assert!(result.models.is_empty());
}

#[tokio::test]
async fn a_service_without_a_model_list_is_limited() {
    let (url, _) = serve("404 Not Found", "", "{}").await;
    assert_eq!(
        probe_models_url(&url, "k").await.status,
        ProfileStatus::Limited
    );
    let (url, _) = serve("200 OK", "", r#"{"data":[]}"#).await;
    assert_eq!(
        probe_models_url(&url, "k").await.status,
        ProfileStatus::Limited
    );
    let (url, _) = serve("200 OK", "", "<html>not json</html>").await;
    assert_eq!(
        probe_models_url(&url, "k").await.status,
        ProfileStatus::Limited
    );
    let (url, _) = serve("429 Too Many Requests", "", "{}").await;
    assert_eq!(
        probe_models_url(&url, "k").await.status,
        ProfileStatus::Limited
    );
}

#[tokio::test]
async fn failures_and_redirects_are_errors() {
    let (url, _) = serve("500 Internal Server Error", "", "{}").await;
    assert_eq!(
        probe_models_url(&url, "k").await.status,
        ProfileStatus::Error
    );
    // A redirect would carry the key to another host, so it is never followed.
    let (url, _) = serve("302 Found", "location: http://127.0.0.1:1/steal\r\n", "").await;
    let redirected = probe_models_url(&url, "k").await;
    assert_eq!(redirected.status, ProfileStatus::Error);
    assert!(redirected.detail.contains("redirect"), "{redirected:?}");
    // Nothing listens on this port.
    let unreachable = probe_models_url("http://127.0.0.1:1/models", "k").await;
    assert_eq!(unreachable.status, ProfileStatus::Error);
}

#[tokio::test]
async fn a_service_with_a_key_check_is_verified_only_when_the_check_passes() {
    // OpenRouter-like: the model list is public, `GET /key` is not.
    let (check, _) = serve(
        "401 Unauthorized",
        "",
        r#"{"error":{"message":"User not found."}}"#,
    )
    .await;
    let (models, _) = serve("200 OK", "", r#"{"data":[{"id":"a/b"}]}"#).await;
    let refused = probe(Some(&check), &models, true, "sk-wrong").await;
    assert_eq!(refused.status, ProfileStatus::Error, "{refused:?}");
    assert!(refused.models.is_empty());

    let (check, _) = serve("200 OK", "", r#"{"data":{"label":"k"}}"#).await;
    let (models, _) = serve("200 OK", "", r#"{"data":[{"id":"a/b"}]}"#).await;
    let ok = probe(Some(&check), &models, true, "sk-right").await;
    assert_eq!(ok.status, ProfileStatus::Verified, "{ok:?}");
    assert_eq!(ok.models, ["a/b"]);
}

#[tokio::test]
async fn a_service_that_lists_models_to_anyone_is_never_more_than_limited() {
    // NVIDIA-like: no call checks the key, so a listed catalogue proves nothing about it.
    let (models, _) = serve(
        "200 OK",
        "",
        r#"{"data":[{"id":"meta/llama"},{"id":"nvidia/nemotron"}]}"#,
    )
    .await;
    let result = probe(None, &models, true, "nvapi-anything").await;
    assert_eq!(result.status, ProfileStatus::Limited, "{result:?}");
    assert_eq!(result.models, ["meta/llama", "nvidia/nemotron"]);
    assert!(result.detail.contains("2 models"), "{result:?}");
    assert!(
        result.detail.contains("checked when a task uses it"),
        "{result:?}"
    );
}

#[tokio::test]
async fn an_oversized_model_list_is_abandoned_not_buffered() {
    // An endless body: if the probe buffered it, this test would never finish (or run out of
    // memory). It must give up once the limit (8 MiB) is passed.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/models", listener.local_addr().unwrap());
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = sent.clone();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let head = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n";
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        let chunk = format!("10000\r\n{}\r\n", "x".repeat(0x10000));
        loop {
            if stream.write_all(chunk.as_bytes()).await.is_err() {
                return;
            }
            counter.fetch_add(0x10000, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        probe_models_url(&url, "k"),
    )
    .await
    .expect("the probe must stop reading at the limit");
    // Stopped a little past the 8 MiB limit, not after reading everything the server could send.
    assert!(sent.load(std::sync::atomic::Ordering::SeqCst) < 64 * 1024 * 1024);
    assert_eq!(result.status, ProfileStatus::Limited, "{:?}", result.detail);
    assert!(result.models.is_empty());
}

#[tokio::test]
async fn a_missing_key_check_endpoint_cannot_verify_a_public_model_list() {
    let (check, _) = serve("404 Not Found", "", "{}").await;
    let (models, _) = serve("200 OK", "", r#"{"data":[{"id":"a/b"}]}"#).await;
    let result = probe(Some(&check), &models, true, "sk-anything").await;
    assert_eq!(result.status, ProfileStatus::Limited, "{result:?}");
    assert_eq!(result.models, ["a/b"]);
}
