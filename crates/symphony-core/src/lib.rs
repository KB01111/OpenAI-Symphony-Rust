pub mod domain;
pub mod error;
pub mod redaction;
pub mod usage;
pub mod workflow;

pub use domain::*;
pub use error::{SymphonyError, SymphonyResult};
pub use redaction::redact_secrets;
pub use usage::{ReportOrigin, ThreadUsageLedger};
pub use workflow::{
    compute_backoff_delay_ms, expand_path_value, render_continuation_prompt,
    render_workflow_prompt, resolve_env_token, AgentConfig, CodexConfig, HooksConfig,
    ServiceConfig, WorkflowDefinition, DEFAULT_PROMPT_TEMPLATE,
};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_workflow_parse_and_front_matter_validation() {
        let raw = "---\ntracker:\n  kind: github\n  required_labels:\n    - symphony\nagent:\n  max_concurrent_agents: 4\n---\nHello {{ issue.identifier }}: {{ issue.title }}";
        let wf = WorkflowDefinition::parse(raw, None).expect("should parse");
        assert_eq!(
            wf.prompt_template,
            "Hello {{ issue.identifier }}: {{ issue.title }}"
        );
        let cfg = ServiceConfig::from_workflow(&wf).expect("valid config");
        assert_eq!(cfg.tracker_kind, "github");
        assert_eq!(cfg.required_labels, vec!["symphony".to_string()]);
        assert_eq!(cfg.agent.max_concurrent_agents, 4);

        let non_map = "---\n- item1\n- item2\n---\nPrompt";
        let err = WorkflowDefinition::parse(non_map, None).unwrap_err();
        assert_eq!(err.code(), "workflow_front_matter_not_a_map");
    }

    #[test]
    fn test_redaction_masks_nested_tracker_credentials() {
        let cfg = ServiceConfig {
            tracker_provider: serde_json::json!({
                "repo": "acme/app",
                "token": "ghp_top",
                "oauth": { "access_token": "nested-secret", "scope": "repo" },
                "credentials": { "user": "bot", "pass": "p" },
                "headers": [{ "authorization": "Bearer x" }],
            "api-key": "hyphenated-secret"
            })
            .as_object()
            .unwrap()
            .clone(),
            ..Default::default()
        };
        let shown = serde_json::to_string(&cfg.redacted_for_display().tracker_provider).unwrap();
        for secret in [
            "ghp_top",
            "nested-secret",
            "\"bot\"",
            "Bearer x",
            "hyphenated-secret",
        ] {
            assert!(!shown.contains(secret), "{secret} leaked in {shown}");
        }
        assert!(shown.contains("acme/app"));
        assert!(shown.contains("\"scope\":\"repo\""));
    }

    #[test]
    fn test_strict_template_rendering() {
        let wf = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: github\n---\nTask {{ issue.identifier }} (attempt={{ attempt }}): {{ issue.title }}",
            None,
        )
        .unwrap();
        let issue = Issue {
            id: "1".to_string(),
            native_ref: None,
            identifier: "GH-42".to_string(),
            title: "Implement feature".to_string(),
            description: Some("Details".to_string()),
            priority: Some(1),
            state: "Todo".to_string(),
            branch_name: None,
            url: None,
            assignee_id: None,
            labels: vec!["symphony".to_string()],
            blocked_by: vec![],
            dispatchable: true,
            created_at: None,
            updated_at: None,
        };

        let rendered = render_workflow_prompt(&wf, &issue, Some(2)).unwrap();
        assert_eq!(rendered, "Task GH-42 (attempt=2): Implement feature");

        let bad_wf = WorkflowDefinition::parse("Hello {{ unknown_var }}", None).unwrap();
        let err = render_workflow_prompt(&bad_wf, &issue, None).unwrap_err();
        assert_eq!(err.code(), "template_render_error");
    }

    #[test]
    fn test_issue_dispatch_sort_order() {
        let i1 = Issue {
            id: "1".into(),
            native_ref: None,
            identifier: "GH-10".into(),
            title: "No priority".into(),
            description: None,
            priority: None,
            state: "Todo".into(),
            branch_name: None,
            url: None,
            assignee_id: None,
            labels: vec![],
            blocked_by: vec![],
            dispatchable: true,
            created_at: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            updated_at: None,
        };
        let mut i2 = i1.clone();
        i2.identifier = "GH-20".into();
        i2.priority = Some(1);
        i2.created_at = Some(Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap());

        let mut i3 = i1.clone();
        i3.identifier = "GH-05".into();
        i3.priority = Some(2);
        i3.created_at = Some(Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0).unwrap());

        let mut list = [i1.clone(), i3.clone(), i2.clone()];
        list.sort_by(|a, b| a.cmp_dispatch_order(b));
        assert_eq!(list[0].identifier, "GH-20");
        assert_eq!(list[1].identifier, "GH-05");
        assert_eq!(list[2].identifier, "GH-10");
    }

    #[test]
    fn test_task_state_transitions() {
        assert!(TaskStatus::Ready.can_transition_to(TaskStatus::Queued));
        assert!(TaskStatus::Queued.can_transition_to(TaskStatus::Running));
        assert!(TaskStatus::Running.can_transition_to(TaskStatus::HumanReview));
        assert!(TaskStatus::HumanReview.can_transition_to(TaskStatus::Completed));
        assert!(!TaskStatus::Backlog.can_transition_to(TaskStatus::Completed));
    }

    #[test]
    fn test_backoff_formula() {
        assert_eq!(compute_backoff_delay_ms(1, true, 300_000), 1_000);
        assert_eq!(compute_backoff_delay_ms(1, false, 300_000), 10_000);
        assert_eq!(compute_backoff_delay_ms(2, false, 300_000), 20_000);
        assert_eq!(compute_backoff_delay_ms(3, false, 300_000), 40_000);
        assert_eq!(compute_backoff_delay_ms(10, false, 300_000), 300_000);
    }

    #[test]
    fn test_secret_redaction() {
        let text = "Token ghp_1234567890abcdef1234567890abcdef and sk-proj-1234567890abcdefghijklm";
        let redacted = redact_secrets(text, &["my_custom_secret".to_string()]);
        assert!(!redacted.contains("ghp_1234"));
        assert!(!redacted.contains("sk-proj-"));
        assert!(redacted.contains("[REDACTED]"));
    }
}
