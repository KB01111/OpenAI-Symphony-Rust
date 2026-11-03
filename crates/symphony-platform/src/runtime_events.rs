//! Following the event stream of a runtime that runs in another process.
//!
//! A desktop window that is attached to a `symphony-cli` runtime does not see that runtime's events
//! the way an embedded runtime's host does, yet an approval request or a failed run should still
//! raise a native notification while the window is in the background. This module reads the
//! runtime's `GET /api/v1/events` stream for that: it finds the runtime through its discovery record
//! ([`RuntimeDiscovery`]), reconnects with a backoff when the runtime restarts, and hands every event
//! to a callback.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use symphony_core::domain::AgentEvent;
use symphony_core::sse::{agent_event, SseParser};
use symphony_core::{SymphonyError, SymphonyResult};

use crate::RuntimeDiscovery;

const FIRST_RETRY: Duration = Duration::from_secs(1);
const LONGEST_RETRY: Duration = Duration::from_secs(30);
/// A connection that lasted this long was a healthy one: the next failure starts the backoff again.
const HEALTHY_AFTER: Duration = Duration::from_secs(10);

fn tracker_error(message: impl Into<String>) -> SymphonyError {
    SymphonyError::IoError(message.into())
}

/// Whether `base_url` points at this machine. The runtime's token is only ever sent there, even
/// though the discovery record it comes from is private to the user.
fn is_loopback_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    }
}

/// Whether the runtime at `base_url` has "start on login" switched on (`GET /api/v1/settings`).
///
/// An `Err` means the answer could not be read; the caller must not take that for "off", or a
/// runtime that is merely busy would get its login entry removed.
pub async fn fetch_start_on_login(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> SymphonyResult<bool> {
    if !is_loopback_url(base_url) {
        return Err(tracker_error(
            "the runtime is not on this machine; its token is not sent",
        ));
    }
    let url = format!("{}/api/v1/settings", base_url.trim_end_matches('/'));
    let response = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| tracker_error(format!("could not reach the runtime's settings: {e}")))?;
    if !response.status().is_success() {
        return Err(tracker_error(format!(
            "the runtime's settings answered {}",
            response.status()
        )));
    }
    let settings: serde_json::Value = response
        .json()
        .await
        .map_err(|e| tracker_error(format!("the runtime's settings are unreadable: {e}")))?;
    settings
        .get("start_background_runtime_on_login")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            tracker_error("the runtime's settings do not say whether Symphony starts on login")
        })
}

/// Read `GET {base_url}/api/v1/events` with `token` and call `on_event` for every event until the
/// stream ends (the runtime stopped, or the token was rotated). Only what happens from now on is
/// asked for: no `Last-Event-ID`, so no history is replayed into a notification.
pub async fn follow_runtime_events(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    mut on_event: impl FnMut(AgentEvent),
) -> SymphonyResult<()> {
    if !is_loopback_url(base_url) {
        return Err(tracker_error(
            "the runtime is not on this machine; its token is not sent",
        ));
    }
    let url = format!("{}/api/v1/events", base_url.trim_end_matches('/'));
    let mut response = client
        .get(&url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await
        .map_err(|e| tracker_error(format!("could not reach the runtime's event stream: {e}")))?;
    if !response.status().is_success() {
        return Err(tracker_error(format!(
            "the runtime's event stream answered {}",
            response.status()
        )));
    }
    let mut parser = SseParser::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| tracker_error(format!("the runtime's event stream broke: {e}")))?
    {
        for frame in parser.push(&chunk) {
            if let Some(event) = agent_event(&frame) {
                on_event(event);
            }
        }
    }
    Ok(())
}

/// Follow the events of the runtime that owns `data_dir` for as long as the process lives.
///
/// The discovery record is read again on every connection, so a runtime that was restarted (new
/// port, new token) is found again; while there is none, or the stream fails, it retries with a
/// backoff of 1 s growing to 30 s.
pub async fn follow_attached_runtime(
    data_dir: PathBuf,
    mut on_event: impl FnMut(AgentEvent) + Send + 'static,
) {
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "Notifications for the attached runtime are unavailable");
            return;
        }
    };
    let mut delay = FIRST_RETRY;
    loop {
        if let Some(found) = RuntimeDiscovery::read_live(&data_dir) {
            let started = Instant::now();
            match follow_runtime_events(&client, &found.base_url, &found.token, &mut on_event).await
            {
                Ok(()) => tracing::debug!("The attached runtime's event stream ended"),
                Err(err) => {
                    tracing::debug!(error = %err, "The attached runtime's event stream failed")
                }
            }
            if started.elapsed() >= HEALTHY_AFTER {
                delay = FIRST_RETRY;
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(LONGEST_RETRY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn event_frame(id: &str, event_type: &str, message: &str) -> String {
        let body = serde_json::json!({
            "id": id, "task_id": "t1", "issue_identifier": "SYM-9", "session_id": null,
            "provider": null, "event_type": event_type, "phase": null, "message": message,
            "payload": null, "timestamp": "2026-11-03T00:00:00Z"
        });
        format!("id: {id}\nevent: {event_type}\ndata: {body}\n\n")
    }

    /// A server that answers one request with `status` and `body`, then closes the connection.
    /// Returns its base URL and the request head it received.
    async fn serve_once(status: &str, body: String) -> (String, Arc<Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let head = Arc::new(Mutex::new(String::new()));
        let seen = head.clone();
        let status = status.to_string();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 2048];
            while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
            }
            *seen.lock().unwrap() = String::from_utf8_lossy(&raw).to_lowercase();
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
            );
            // Written in small pieces: frames arrive split across reads, as on a real connection.
            for piece in reply.as_bytes().chunks(23) {
                let _ = stream.write_all(piece).await;
                let _ = stream.flush().await;
            }
            let _ = stream.shutdown().await;
        });
        (base, head)
    }

    #[tokio::test]
    async fn events_are_read_in_order_with_the_token_and_without_asking_for_history() {
        let stream = format!(
            "event: ready\ndata: {{\"api_version\":3}}\n\n: keep-alive\n\n{}{}",
            event_frame("e1", "approval/requested", "Run cargo test"),
            event_frame("e2", "attempt/failed", "Agent turn failed")
        );
        let (base, head) = serve_once("200 OK", stream).await;
        let client = reqwest::Client::new();
        let mut seen = Vec::new();

        follow_runtime_events(&client, &base, "local-token", |e| seen.push(e))
            .await
            .unwrap();

        let types: Vec<_> = seen.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(types, ["approval/requested", "attempt/failed"]);
        assert_eq!(seen[0].issue_identifier.as_deref(), Some("SYM-9"));
        let head = head.lock().unwrap().clone();
        assert!(head.starts_with("get /api/v1/events "), "{head}");
        assert!(head.contains("authorization: bearer local-token"), "{head}");
        assert!(head.contains("accept: text/event-stream"), "{head}");
        assert!(!head.contains("last-event-id"), "no history replay: {head}");
    }

    #[tokio::test]
    async fn the_login_setting_is_read_from_the_runtime_and_unreadable_answers_are_errors() {
        let client = reqwest::Client::new();
        let (base, head) = serve_once(
            "200 OK",
            serde_json::json!({ "start_background_runtime_on_login": true }).to_string(),
        )
        .await;
        assert!(fetch_start_on_login(&client, &base, "local-token")
            .await
            .unwrap());
        let head = head.lock().unwrap().clone();
        assert!(head.starts_with("get /api/v1/settings "), "{head}");
        assert!(head.contains("authorization: bearer local-token"), "{head}");

        // Off is an answer; everything else is "unknown", never "off".
        let (base, _) = serve_once(
            "200 OK",
            serde_json::json!({ "start_background_runtime_on_login": false }).to_string(),
        )
        .await;
        assert!(!fetch_start_on_login(&client, &base, "t").await.unwrap());
        let (base, _) = serve_once("500 Internal Server Error", String::new()).await;
        assert!(fetch_start_on_login(&client, &base, "t").await.is_err());
        let (base, _) = serve_once("200 OK", "{}".to_string()).await;
        assert!(fetch_start_on_login(&client, &base, "t").await.is_err());
        let (base, _) = serve_once("200 OK", "not json".to_string()).await;
        assert!(fetch_start_on_login(&client, &base, "t").await.is_err());
        // The token is not sent off this machine.
        assert!(
            fetch_start_on_login(&client, "http://example.com:4780", "t")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_refused_stream_is_an_error_and_delivers_nothing() {
        let (base, _) = serve_once("401 Unauthorized", String::new()).await;
        let mut delivered = 0;
        let err =
            follow_runtime_events(&reqwest::Client::new(), &base, "stale", |_| delivered += 1)
                .await
                .unwrap_err();
        assert!(err.to_string().contains("401"), "{err}");
        assert_eq!(delivered, 0);
    }

    #[tokio::test]
    async fn following_an_absent_runtime_just_keeps_trying() {
        // Spawned the way the desktop host spawns it (`Send + 'static`), with nothing published.
        let dir = tempfile::tempdir().unwrap();
        let following = tokio::spawn(follow_attached_runtime(
            dir.path().to_path_buf(),
            |_event| {},
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !following.is_finished(),
            "it waits for a runtime instead of giving up"
        );
        following.abort();
    }

    #[tokio::test]
    async fn the_token_is_only_ever_sent_to_this_machine() {
        for remote in [
            "http://example.com:4780",
            "https://192.0.2.7:4780",
            "http://10.0.0.5:4780",
            "not a url",
            "",
        ] {
            let err = follow_runtime_events(&reqwest::Client::new(), remote, "secret", |_| {})
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("not on this machine"),
                "{remote}: {err}"
            );
        }
        for local in [
            "http://127.0.0.1:4780",
            "http://localhost:4780",
            "http://[::1]:4780",
        ] {
            assert!(is_loopback_url(local), "{local}");
        }
    }
}
