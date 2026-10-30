pub mod domain;
pub mod error;
pub mod redaction;
pub mod tool_calls;
pub mod usage;
pub mod workflow;

pub use domain::*;
pub use error::{SymphonyError, SymphonyResult};
pub use redaction::{detect_secrets_in_added_lines, redact_json_strings, redact_secrets};
pub use tool_calls::{
    ToolCallKind, ToolCallRecord, ToolCallStatus, ToolCallUpdate, MAX_TOOL_OUTPUT_BYTES,
};
pub use usage::{ReportOrigin, ThreadUsageLedger};
pub use workflow::{
    compute_backoff_delay_ms, expand_path_value, render_continuation_prompt,
    render_workflow_prompt, resolve_env_token, AgentConfig, CodeSandboxConfig, CodexConfig,
    HooksConfig, ServiceConfig, WorkflowDefinition, DEFAULT_PROMPT_TEMPLATE,
};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn passing_validation() -> ValidationResult {
        ValidationResult {
            id: "v1".into(),
            task_id: "t1".into(),
            attempt_number: 1,
            command: "cargo test".into(),
            exit_code: 0,
            passed: true,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 1,
            executed_at: Utc::now(),
        }
    }

    #[test]
    fn merge_readiness_never_claims_unverified_ci_or_merge_state() {
        let report = MergeReadinessReport::evaluate(
            ConflictCheck::NotChecked,
            "none",
            &[passing_validation()],
            &[],
            2,
        );
        let evidence = report.evidence.join("\n");
        assert!(!evidence.contains("CI checks passed"), "{evidence}");
        assert!(!evidence.contains("without conflicts"), "{evidence}");
        assert!(evidence.contains("could not be checked"), "{evidence}");
        assert!(evidence.contains("No remote CI run"), "{evidence}");
        assert!(report.blockers.is_empty());
        // An unverified merge is never the top rating, with everything else in order.
        assert_eq!(report.score, 4);
        assert_ne!(report.label, "Ready for Human Approval & Merge");
    }

    #[test]
    fn merge_readiness_penalises_conflicts_and_failed_ci() {
        let clean = MergeReadinessReport::evaluate(
            ConflictCheck::Clean,
            "passed",
            &[passing_validation()],
            &[],
            1,
        );
        assert_eq!(clean.score, 5);
        assert!(clean
            .evidence
            .iter()
            .any(|e| e.contains("Remote CI checks passed")));

        let conflicted = MergeReadinessReport::evaluate(
            ConflictCheck::Conflicts,
            "failed",
            &[passing_validation()],
            &[],
            1,
        );
        assert_eq!(conflicted.score, 1);
        assert_eq!(conflicted.blockers.len(), 2);
    }

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

    #[test]
    fn a_codesandbox_token_in_an_added_line_is_found_by_the_credential_scan() {
        // Log redaction is not enough: a committed key must also block publishing the branch.
        // The fixture is assembled at run time: written out whole, this very file would trip the scan
        // whenever a task branch of this repository adds it.
        let key = format!("csb_v1_{}", "AbCdEf0123456789AbCdEf0123456789");
        let diff = format!("+CSB_API_KEY={key}\n context line\n");
        let findings = detect_secrets_in_added_lines(&diff);
        assert!(findings.contains(&"CodeSandbox API token"), "{findings:?}");
        // Removed and unchanged lines are not findings.
        let removed = format!("-CSB_API_KEY={key}\n");
        assert!(detect_secrets_in_added_lines(&removed).is_empty());
    }

    #[test]
    fn codesandbox_secrets_are_redacted() {
        let text = format!(
            "key csb_v1_{} in the log",
            "AbCdEf0123456789AbCdEf0123456789"
        );
        let redacted = redact_secrets(&text, &[]);
        assert!(!redacted.contains("csb_v1_"), "{redacted}");
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn cloud_agent_and_codesandbox_provider_names_round_trip() {
        use std::str::FromStr;
        for agent in [
            CloudAgentKind::MiniSweAgent,
            CloudAgentKind::Pi,
            CloudAgentKind::PiDurable,
        ] {
            assert_eq!(CloudAgentKind::from_str(agent.as_str()).unwrap(), agent);
            // The wire format is the same snake_case name.
            assert_eq!(
                serde_json::to_value(agent).unwrap(),
                serde_json::json!(agent.as_str())
            );
        }
        // Friendly spellings are accepted; mini-swe-agent is the default.
        assert_eq!(
            CloudAgentKind::from_str("Pi-Durable").unwrap(),
            CloudAgentKind::PiDurable
        );
        assert_eq!(
            CloudAgentKind::from_str("mini").unwrap(),
            CloudAgentKind::MiniSweAgent
        );
        assert_eq!(CloudAgentKind::default(), CloudAgentKind::MiniSweAgent);
        assert!(CloudAgentKind::from_str("gpt-engineer").is_err());

        assert_eq!(
            ProviderKind::from_str("codesandbox").unwrap(),
            ProviderKind::CodeSandbox
        );
        assert_eq!(ProviderKind::CodeSandbox.as_str(), "codesandbox");
        assert_eq!(
            serde_json::to_value(ProviderKind::CodeSandbox).unwrap(),
            serde_json::json!("codesandbox")
        );
        // Persisted spellings from the other direction still parse.
        assert_eq!(
            serde_json::from_value::<ProviderKind>(serde_json::json!("code_sandbox")).unwrap(),
            ProviderKind::CodeSandbox
        );
        let capabilities = ProviderKind::CodeSandbox.default_capabilities();
        assert!(capabilities.supports_session_resume && capabilities.supports_cancel);
        assert!(
            !capabilities.supports_approvals,
            "a sandboxed agent has no approval round trip"
        );
    }

    #[test]
    fn the_runner_path_expands_a_home_directory_like_other_workflow_paths() {
        let raw = "---\ncodesandbox:\n  runner_path: ~/symphony/bridge.mjs\n---\nx";
        let cfg =
            ServiceConfig::from_workflow(&WorkflowDefinition::parse(raw, None).unwrap()).unwrap();
        let path = cfg.codesandbox.runner_path.unwrap();
        assert!(!path.starts_with('~'), "{path}");
        assert!(
            path.replace('\\', "/").ends_with("/symphony/bridge.mjs"),
            "{path}"
        );
        assert!(std::path::Path::new(&path).is_absolute(), "{path}");
    }

    #[test]
    fn an_unset_runner_path_variable_means_no_override() {
        // Resolved before the empty check: it must not become an explicit path (the temp-dir fallback of
        // `expand_path_value`) that the launch would then treat as authoritative.
        let raw =
            "---\ncodesandbox:\n  runner_path: $SYMPHONY_TEST_RUNNER_PATH_SURELY_UNSET\n---\nx";
        let cfg =
            ServiceConfig::from_workflow(&WorkflowDefinition::parse(raw, None).unwrap()).unwrap();
        assert_eq!(cfg.codesandbox.runner_path, None);
    }

    #[test]
    fn the_codesandbox_section_is_optional_and_validated() {
        let none =
            WorkflowDefinition::parse("---\ntracker:\n  kind: github\n---\nx", None).unwrap();
        let cfg = ServiceConfig::from_workflow(&none).unwrap();
        assert_eq!(cfg.codesandbox.default_agent, CloudAgentKind::MiniSweAgent);
        assert_eq!(cfg.codesandbox.template_id, "symphony-agents");
        assert_eq!(cfg.codesandbox.vm_tier, "small");
        cfg.validate_for_dispatch().unwrap();

        let raw = "---\ncodesandbox:\n  template_id: tpl-123\n  default_agent: pi_durable\n  vm_tier: Medium\n  hibernation_timeout_secs: 600\n  cost_limit_usd: 1.5\n  runner_path: C:/runner/bridge.mjs\n---\nx";
        let cfg =
            ServiceConfig::from_workflow(&WorkflowDefinition::parse(raw, None).unwrap()).unwrap();
        assert_eq!(cfg.codesandbox.template_id, "tpl-123");
        assert_eq!(cfg.codesandbox.default_agent, CloudAgentKind::PiDurable);
        // Tier names are case-insensitive in the file and normalised.
        assert_eq!(cfg.codesandbox.vm_tier, "medium");
        assert_eq!(cfg.codesandbox.hibernation_timeout_secs, 600);
        assert!((cfg.codesandbox.cost_limit_usd - 1.5).abs() < f64::EPSILON);
        assert_eq!(
            cfg.codesandbox.runner_path.as_deref(),
            Some("C:/runner/bridge.mjs")
        );
        cfg.validate_for_dispatch().unwrap();

        // An agent that does not exist is a configuration error, not a silent fallback.
        let bad_agent = "---\ncodesandbox:\n  default_agent: gpt-engineer\n---\nx";
        assert!(
            ServiceConfig::from_workflow(&WorkflowDefinition::parse(bad_agent, None).unwrap())
                .is_err()
        );

        // So is a VM tier the SDK does not have, found before any sandbox is started.
        let bad_tier = "---\ncodesandbox:\n  vm_tier: gigantic\n---\nx";
        let cfg = ServiceConfig::from_workflow(&WorkflowDefinition::parse(bad_tier, None).unwrap())
            .unwrap();
        let err = cfg.validate_for_dispatch().unwrap_err().to_string();
        assert!(err.contains("gigantic") && err.contains("pico"), "{err}");
    }
}
