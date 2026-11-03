//! Reading the runtime's Server-Sent Events stream (`GET /api/v1/events`) outside a browser.
//!
//! The desktop uses this while it is attached to a runtime that runs on its own (`symphony-cli`),
//! to raise the same native notifications it raises for its own events. A browser has `EventSource`;
//! this is the small part of the SSE format the runtime actually sends: `id:`, `event:` and `data:`
//! fields, comment lines (keep-alives) and a blank line after each frame. Lines end in `\n` or
//! `\r\n`.

use crate::domain::AgentEvent;

/// One dispatched event of the stream.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseFrame {
    pub id: Option<String>,
    pub event: Option<String>,
    /// The `data:` lines, joined with `\n`.
    pub data: String,
}

/// Incremental parser: feed it the bytes as they arrive, in chunks of any size, and it returns the
/// frames each chunk completed. A frame is only returned once its terminating blank line has been
/// seen, so a chunk that ends mid-frame (or mid-character) is carried over to the next.
#[derive(Debug, Default)]
pub struct SseParser {
    /// Bytes of the line being received.
    pending: Vec<u8>,
    frame: SseFrame,
    has_data: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        let mut frames = Vec::new();
        for &byte in chunk {
            if byte != b'\n' {
                self.pending.push(byte);
                continue;
            }
            // A complete line (its UTF-8 sequences are whole: they cannot contain `\n`).
            let mut line = std::mem::take(&mut self.pending);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if let Some(frame) = self.line(&String::from_utf8_lossy(&line)) {
                frames.push(frame);
            }
        }
        frames
    }

    fn line(&mut self, line: &str) -> Option<SseFrame> {
        if line.is_empty() {
            // End of a frame. One without data is not an event (the format's own rule).
            let frame = std::mem::take(&mut self.frame);
            return std::mem::take(&mut self.has_data).then_some(frame);
        }
        // A line starting with `:` is a comment; the runtime sends them as keep-alives.
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.frame.event = Some(value.to_string()),
            "id" if !value.contains('\0') => self.frame.id = Some(value.to_string()),
            "data" => {
                if self.has_data {
                    self.frame.data.push('\n');
                }
                self.frame.data.push_str(value);
                self.has_data = true;
            }
            // `retry:` and unknown fields are ignored.
            _ => {}
        }
        None
    }
}

/// The [`AgentEvent`] a frame carries. `ready` and `resync` are control frames of the stream, and a
/// frame whose data is not an event is skipped, not an error: one odd frame must not end the stream.
pub fn agent_event(frame: &SseFrame) -> Option<AgentEvent> {
    match frame.event.as_deref() {
        Some("ready" | "resync") => None,
        _ => serde_json::from_str(&frame.data).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_json(id: &str, event_type: &str, message: &str) -> String {
        serde_json::json!({
            "id": id,
            "task_id": "t1",
            "issue_identifier": "SYM-1",
            "session_id": null,
            "provider": null,
            "event_type": event_type,
            "phase": null,
            "message": message,
            "payload": null,
            "timestamp": "2026-11-03T00:00:00Z"
        })
        .to_string()
    }

    fn frame_text(id: &str, event_type: &str, message: &str) -> String {
        format!(
            "id: {id}\nevent: {event_type}\ndata: {}\n\n",
            event_json(id, event_type, message)
        )
    }

    #[test]
    fn a_frame_is_returned_with_its_fields_once_its_blank_line_arrives() {
        let mut parser = SseParser::new();
        assert!(parser
            .push(b"id: 7\nevent: approval/requested\ndata: {\"a\":1}\n")
            .is_empty());
        let frames = parser.push(b"\n");
        assert_eq!(
            frames,
            vec![SseFrame {
                id: Some("7".into()),
                event: Some("approval/requested".into()),
                data: "{\"a\":1}".into()
            }]
        );
    }

    #[test]
    fn any_chunking_gives_the_same_frames_even_inside_a_multibyte_character() {
        let text = format!(
            "{}{}",
            frame_text("1", "attempt/failed", "Fehler: größer än väntat ✓"),
            frame_text("2", "approval/requested", "Run tests")
        );
        let whole = SseParser::new().push(text.as_bytes());
        assert_eq!(whole.len(), 2);
        for size in [1, 2, 3, 5, 7, 64] {
            let mut parser = SseParser::new();
            let mut frames = Vec::new();
            for chunk in text.as_bytes().chunks(size) {
                frames.extend(parser.push(chunk));
            }
            assert_eq!(frames, whole, "chunk size {size}");
        }
        assert_eq!(
            agent_event(&whole[0]).unwrap().message,
            "Fehler: größer än väntat ✓"
        );
    }

    #[test]
    fn crlf_line_endings_comments_and_multiline_data_are_handled() {
        let mut parser = SseParser::new();
        let frames = parser.push(
            b": keep-alive\r\n\r\nevent: x\r\ndata: first\r\ndata: second\r\nretry: 3000\r\nfoo: bar\r\n\r\n",
        );
        assert_eq!(
            frames,
            vec![SseFrame {
                id: None,
                event: Some("x".into()),
                data: "first\nsecond".into()
            }],
            "the keep-alive comment alone is not an event"
        );
    }

    #[test]
    fn a_frame_without_data_is_not_an_event() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"event: x\n\n").is_empty());
        // And the next frame does not inherit its fields.
        let frames = parser.push(b"data: y\n\n");
        assert_eq!(frames[0].event, None);
        assert_eq!(frames[0].data, "y");
    }

    #[test]
    fn control_frames_and_malformed_data_yield_no_event() {
        let ready = SseFrame {
            event: Some("ready".into()),
            data: "{\"api_version\":3,\"server_time\":\"2026-11-03T00:00:00Z\"}".into(),
            ..Default::default()
        };
        let resync = SseFrame {
            event: Some("resync".into()),
            data: "{}".into(),
            ..Default::default()
        };
        let garbage = SseFrame {
            event: Some("attempt/failed".into()),
            data: "not json".into(),
            ..Default::default()
        };
        assert!(agent_event(&ready).is_none());
        assert!(agent_event(&resync).is_none());
        assert!(agent_event(&garbage).is_none());
    }

    #[test]
    fn the_event_the_runtime_sends_round_trips() {
        let mut parser = SseParser::new();
        let frames =
            parser.push(frame_text("e1", "approval/requested", "Run cargo test").as_bytes());
        let event = agent_event(&frames[0]).expect("an agent event");
        assert_eq!(event.id, "e1");
        assert_eq!(event.event_type, "approval/requested");
        assert_eq!(event.issue_identifier.as_deref(), Some("SYM-1"));
        assert_eq!(event.message, "Run cargo test");
    }
}
