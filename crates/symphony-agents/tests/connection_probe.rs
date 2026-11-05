//! The connection test against a stand-in model service on loopback.

use symphony_agents::connection_probe::probe_models_url;
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
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap();
        stream.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&buf[..n]).to_string()
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
