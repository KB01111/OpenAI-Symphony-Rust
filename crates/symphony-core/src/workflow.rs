use liquid::model::Value as LiquidValue;
use liquid::ParserBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use ts_rs::TS;

use crate::domain::Issue;
use crate::error::{SymphonyError, SymphonyResult};

pub const DEFAULT_PROMPT_TEMPLATE: &str =
    "You are working on an issue from the project tracker.\n\nIssue: {{ issue.identifier }} - {{ issue.title }}\n\n{{ issue.description }}";

/// Parsed `WORKFLOW.md` payload (SPEC.md Section 5.1)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub config: Map<String, Value>,
    pub prompt_template: String,
    pub source_path: Option<PathBuf>,
    /// Original document text (front matter + body) so editors can round-trip the full file.
    #[serde(default)]
    pub raw_source: String,
}

impl WorkflowDefinition {
    /// Parse a `WORKFLOW.md` string into YAML front matter map and trimmed Markdown body (SPEC.md Section 5.2)
    pub fn parse(raw: &str, source_path: Option<PathBuf>) -> SymphonyResult<Self> {
        let normalized = raw.replace("\r\n", "\n");
        if let Some(rest) = normalized.strip_prefix("---\n") {
            if let Some(end_idx) = rest.find("\n---\n") {
                let yaml_str = &rest[..end_idx];
                let body = &rest[end_idx + 5..];
                let config = parse_yaml_front_matter(yaml_str)?;
                return Ok(Self {
                    config,
                    prompt_template: body.trim().to_string(),
                    source_path,
                    raw_source: raw.to_string(),
                });
            } else if let Some(yaml_str) = rest.strip_suffix("\n---") {
                let config = parse_yaml_front_matter(yaml_str)?;
                return Ok(Self {
                    config,
                    prompt_template: String::new(),
                    source_path,
                    raw_source: raw.to_string(),
                });
            } else {
                return Err(SymphonyError::WorkflowParseError(
                    "unclosed YAML front matter delimiter '---'".to_string(),
                ));
            }
        } else if normalized == "---" {
            return Err(SymphonyError::WorkflowParseError(
                "unclosed YAML front matter delimiter '---'".to_string(),
            ));
        }

        Ok(Self {
            config: Map::new(),
            prompt_template: normalized.trim().to_string(),
            source_path,
            raw_source: raw.to_string(),
        })
    }

    /// Load a `WORKFLOW.md` file from disk (SPEC.md Section 5.1)
    pub fn load_from_path(path: impl AsRef<Path>) -> SymphonyResult<Self> {
        let path_ref = path.as_ref();
        if !path_ref.exists() {
            return Err(SymphonyError::MissingWorkflowFile(
                path_ref.display().to_string(),
            ));
        }
        let content = std::fs::read_to_string(path_ref).map_err(|e| {
            SymphonyError::MissingWorkflowFile(format!("{}: {e}", path_ref.display()))
        })?;
        Self::parse(&content, Some(path_ref.to_path_buf()))
    }
}

fn parse_yaml_front_matter(yaml_str: &str) -> SymphonyResult<Map<String, Value>> {
    if yaml_str.trim().is_empty() {
        return Ok(Map::new());
    }
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(yaml_str)
        .map_err(|e| SymphonyError::WorkflowParseError(e.to_string()))?;
    let json_val: Value = serde_json::to_value(yaml_val)
        .map_err(|e| SymphonyError::WorkflowParseError(e.to_string()))?;
    match json_val {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        _ => Err(SymphonyError::WorkflowFrontMatterNotAMap),
    }
}

/// Typed view over `WorkflowDefinition.config` (SPEC.md Section 5.3 & Section 6)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ServiceConfig {
    pub tracker_kind: String,
    pub tracker_provider: Map<String, Value>,
    pub required_labels: Vec<String>,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub polling_interval_ms: u64,
    #[ts(type = "string")]
    pub workspace_root: PathBuf,
    pub hooks: HooksConfig,
    pub agent: AgentConfig,
    pub codex: CodexConfig,
    pub server_port: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HooksConfig {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AgentConfig {
    pub max_concurrent_agents: u32,
    pub max_turns: u32,
    pub max_retry_backoff_ms: u64,
    pub max_concurrent_agents_by_state: HashMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CodexConfig {
    pub command: String,
    pub approval_policy: Value,
    pub thread_sandbox: String,
    pub turn_sandbox_policy: Option<Value>,
    pub turn_timeout_ms: u64,
    pub read_timeout_ms: u64,
    pub stall_timeout_ms: i64,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        let default_ws = std::env::temp_dir().join("symphony_workspaces");
        Self {
            tracker_kind: "github".to_string(),
            tracker_provider: Map::new(),
            required_labels: Vec::new(),
            active_states: vec![
                "Todo".to_string(),
                "In Progress".to_string(),
                "open".to_string(),
            ],
            terminal_states: vec![
                "Closed".to_string(),
                "Cancelled".to_string(),
                "Canceled".to_string(),
                "Duplicate".to_string(),
                "Done".to_string(),
                "closed".to_string(),
            ],
            polling_interval_ms: 30_000,
            workspace_root: default_ws,
            hooks: HooksConfig {
                after_create: None,
                before_run: None,
                after_run: None,
                before_remove: None,
                timeout_ms: 60_000,
            },
            agent: AgentConfig {
                max_concurrent_agents: 10,
                max_turns: 20,
                max_retry_backoff_ms: 300_000,
                max_concurrent_agents_by_state: HashMap::new(),
            },
            codex: CodexConfig {
                command: "codex app-server".to_string(),
                approval_policy: Value::Object({
                    let mut m = Map::new();
                    let mut reject = Map::new();
                    reject.insert("sandbox_approval".to_string(), Value::Bool(true));
                    reject.insert("rules".to_string(), Value::Bool(true));
                    reject.insert("mcp_elicitations".to_string(), Value::Bool(true));
                    m.insert("reject".to_string(), Value::Object(reject));
                    m
                }),
                thread_sandbox: "workspace-write".to_string(),
                turn_sandbox_policy: None,
                turn_timeout_ms: 3_600_000,
                read_timeout_ms: 5_000,
                stall_timeout_ms: 300_000,
            },
            server_port: None,
        }
    }
}

impl ServiceConfig {
    /// Copy of the configuration that is safe to return over the API or print:
    /// tracker credential values (`token`, `api_key`, `secret`, `password`) are masked.
    pub fn redacted_for_display(&self) -> Self {
        let mut copy = self.clone();
        for (key, value) in copy.tracker_provider.iter_mut() {
            let lower = key.to_lowercase();
            let sensitive = ["token", "api_key", "apikey", "secret", "password"]
                .iter()
                .any(|needle| lower.contains(needle));
            if sensitive {
                if let Value::String(s) = value {
                    if !s.is_empty() {
                        *value = Value::String("[redacted]".to_string());
                    }
                }
            }
        }
        copy
    }

    /// Extract typed `ServiceConfig` from a `WorkflowDefinition` (SPEC.md Section 5.3)
    pub fn from_workflow(wf: &WorkflowDefinition) -> SymphonyResult<Self> {
        let mut cfg = Self::default();
        let root = &wf.config;

        if let Some(Value::Object(tracker)) = root.get("tracker") {
            if let Some(Value::String(k)) = tracker.get("kind") {
                if !k.trim().is_empty() {
                    cfg.tracker_kind = resolve_env_token(k);
                }
            }
            if let Some(provider_val) = tracker.get("provider") {
                match provider_val {
                    Value::Object(map) => {
                        cfg.tracker_provider = resolve_env_in_map(map);
                    }
                    Value::Null => {}
                    _ => {
                        return Err(SymphonyError::InvalidConfig(
                            "tracker.provider must be a map/object".to_string(),
                        ))
                    }
                }
            }
            // Also support flat fields like `tracker.repo`, `tracker.project_slug`, `tracker.api_key`
            for flat_key in [
                "repo",
                "project_slug",
                "api_key",
                "token",
                "endpoint",
                "api_url",
                "assignee",
            ] {
                if let Some(v) = tracker.get(flat_key) {
                    if !cfg.tracker_provider.contains_key(flat_key) {
                        cfg.tracker_provider
                            .insert(flat_key.to_string(), resolve_env_in_value(v));
                    }
                }
            }
            if let Some(labels) = tracker.get("required_labels") {
                cfg.required_labels = parse_string_list(labels);
            }
            if let Some(active) = tracker.get("active_states") {
                let parsed = parse_string_list(active);
                if !parsed.is_empty() {
                    cfg.active_states = parsed;
                }
            }
            if let Some(term) = tracker.get("terminal_states") {
                let parsed = parse_string_list(term);
                if !parsed.is_empty() {
                    cfg.terminal_states = parsed;
                }
            }
        }

        if let Some(Value::Object(polling)) = root.get("polling") {
            if let Some(v) = polling.get("interval_ms").and_then(parse_u64_flexible) {
                if v > 0 {
                    cfg.polling_interval_ms = v;
                }
            }
        }

        if let Some(Value::Object(ws)) = root.get("workspace") {
            if let Some(Value::String(raw_root)) = ws.get("root") {
                let expanded = expand_path_value(raw_root, wf.source_path.as_deref());
                if !expanded.as_os_str().is_empty() {
                    cfg.workspace_root = expanded;
                }
            }
        }

        if let Some(Value::Object(hooks)) = root.get("hooks") {
            cfg.hooks.after_create = hooks
                .get("after_create")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            cfg.hooks.before_run = hooks
                .get("before_run")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            cfg.hooks.after_run = hooks
                .get("after_run")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            cfg.hooks.before_remove = hooks
                .get("before_remove")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if let Some(timeout) = hooks.get("timeout_ms").and_then(parse_u64_flexible) {
                if timeout > 0 {
                    cfg.hooks.timeout_ms = timeout;
                }
            }
        }

        if let Some(Value::Object(agent)) = root.get("agent") {
            if let Some(v) = agent
                .get("max_concurrent_agents")
                .and_then(parse_u64_flexible)
            {
                if v > 0 {
                    cfg.agent.max_concurrent_agents = v as u32;
                }
            }
            if let Some(v) = agent.get("max_turns").and_then(parse_u64_flexible) {
                if v > 0 {
                    cfg.agent.max_turns = v as u32;
                }
            }
            if let Some(v) = agent
                .get("max_retry_backoff_ms")
                .and_then(parse_u64_flexible)
            {
                if v > 0 {
                    cfg.agent.max_retry_backoff_ms = v;
                }
            }
            if let Some(Value::Object(by_state)) = agent.get("max_concurrent_agents_by_state") {
                for (k, val) in by_state {
                    if let Some(n) = parse_u64_flexible(val) {
                        if n > 0 {
                            cfg.agent
                                .max_concurrent_agents_by_state
                                .insert(k.trim().to_lowercase(), n as u32);
                        }
                    }
                }
            }
        }

        if let Some(Value::Object(codex)) = root.get("codex") {
            if let Some(Value::String(cmd)) = codex.get("command") {
                if !cmd.trim().is_empty() {
                    cfg.codex.command = cmd.trim().to_string();
                }
            }
            if let Some(ap) = codex.get("approval_policy") {
                if !ap.is_null() {
                    cfg.codex.approval_policy = ap.clone();
                }
            }
            if let Some(Value::String(sb)) = codex.get("thread_sandbox") {
                if !sb.trim().is_empty() {
                    cfg.codex.thread_sandbox = sb.trim().to_string();
                }
            }
            if let Some(tsp) = codex.get("turn_sandbox_policy") {
                if !tsp.is_null() {
                    cfg.codex.turn_sandbox_policy = Some(tsp.clone());
                }
            }
            if let Some(v) = codex.get("turn_timeout_ms").and_then(parse_u64_flexible) {
                if v > 0 {
                    cfg.codex.turn_timeout_ms = v;
                }
            }
            if let Some(v) = codex.get("read_timeout_ms").and_then(parse_u64_flexible) {
                if v > 0 {
                    cfg.codex.read_timeout_ms = v;
                }
            }
            if let Some(v) = codex.get("stall_timeout_ms").and_then(parse_i64_flexible) {
                cfg.codex.stall_timeout_ms = v;
            }
        }

        if let Some(Value::Object(server)) = root.get("server") {
            if let Some(port) = server.get("port").and_then(parse_u64_flexible) {
                if port <= u16::MAX as u64 {
                    cfg.server_port = Some(port as u16);
                }
            }
        }

        Ok(cfg)
    }

    /// Dispatch preflight validation (SPEC.md Section 6.3)
    pub fn validate_for_dispatch(&self) -> SymphonyResult<()> {
        if self.tracker_kind.trim().is_empty() {
            return Err(SymphonyError::InvalidConfig(
                "tracker.kind is required".to_string(),
            ));
        }
        if self.codex.command.trim().is_empty() {
            return Err(SymphonyError::InvalidConfig(
                "codex.command must be non-empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Check if an issue state is in `active_states` (case-insensitive)
    pub fn is_active_state(&self, state: &str) -> bool {
        let norm = state.trim().to_lowercase();
        self.active_states
            .iter()
            .any(|s| s.trim().to_lowercase() == norm)
    }

    /// Check if an issue state is in `terminal_states` (case-insensitive)
    pub fn is_terminal_state(&self, state: &str) -> bool {
        let norm = state.trim().to_lowercase();
        self.terminal_states
            .iter()
            .any(|s| s.trim().to_lowercase() == norm)
    }
}

pub fn resolve_env_token(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(var_name) = trimmed.strip_prefix('$') {
        if !var_name.is_empty()
            && var_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return std::env::var(var_name).unwrap_or_default();
        }
    }
    raw.to_string()
}

fn resolve_env_in_value(val: &Value) -> Value {
    match val {
        Value::String(s) => Value::String(resolve_env_token(s)),
        Value::Object(m) => Value::Object(resolve_env_in_map(m)),
        Value::Array(arr) => Value::Array(arr.iter().map(resolve_env_in_value).collect()),
        other => other.clone(),
    }
}

fn resolve_env_in_map(map: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in map {
        out.insert(k.clone(), resolve_env_in_value(v));
    }
    out
}

fn parse_string_list(val: &Value) -> Vec<String> {
    match val {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        Value::String(s) => s
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_u64_flexible(val: &Value) -> Option<u64> {
    match val {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => resolve_env_token(s).trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn parse_i64_flexible(val: &Value) -> Option<i64> {
    match val {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => resolve_env_token(s).trim().parse::<i64>().ok(),
        _ => None,
    }
}

pub fn expand_path_value(raw: &str, workflow_file: Option<&Path>) -> PathBuf {
    let resolved = resolve_env_token(raw);
    let trimmed = resolved.trim();
    if trimmed.is_empty() {
        return std::env::temp_dir().join("symphony_workspaces");
    }

    let path = if trimmed == "~" {
        dirs::home_dir().unwrap_or_else(std::env::temp_dir)
    } else if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(rest)
    } else {
        PathBuf::from(trimmed)
    };

    if path.is_relative() {
        if let Some(wf_path) = workflow_file {
            if let Some(parent) = wf_path.parent() {
                return parent.join(path);
            }
        }
    }
    path
}

/// Render the workflow prompt using strict Liquid rules (SPEC.md Section 5.4 & Section 12)
/// Unknown variables or unknown filters fail immediately.
pub fn render_workflow_prompt(
    workflow: &WorkflowDefinition,
    issue: &Issue,
    attempt: Option<u32>,
) -> SymphonyResult<String> {
    let template_src = if workflow.prompt_template.trim().is_empty() {
        DEFAULT_PROMPT_TEMPLATE
    } else {
        workflow.prompt_template.as_str()
    };

    // Strict check for unknown top-level variables referenced in `{{ ... }}` or `{% ... %}`
    validate_template_variables(template_src, issue)?;

    let parser = ParserBuilder::with_stdlib()
        .build()
        .map_err(|e| SymphonyError::TemplateParseError(e.to_string()))?;

    let template = parser
        .parse(template_src)
        .map_err(|e| SymphonyError::TemplateParseError(e.to_string()))?;

    let issue_json = serde_json::to_value(issue)
        .map_err(|e| SymphonyError::TemplateRenderError(e.to_string()))?;
    let issue_liquid = json_to_liquid(&issue_json);

    let attempt_liquid = match attempt {
        Some(n) => LiquidValue::scalar(n as i64),
        None => LiquidValue::Nil,
    };

    let globals = liquid::object!({
        "issue": issue_liquid,
        "attempt": attempt_liquid,
    });

    template
        .render(&globals)
        .map_err(|e| SymphonyError::TemplateRenderError(e.to_string()))
}

fn validate_template_variables(template_src: &str, issue: &Issue) -> SymphonyResult<()> {
    let expr_re = regex::Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_.]*)\s*(?:\|[^}]*)?\}\}")
        .expect("valid regex");
    let issue_json = serde_json::to_value(issue)
        .map_err(|e| SymphonyError::TemplateRenderError(e.to_string()))?;
    let allowed_issue_fields = issue_json.as_object();

    for caps in expr_re.captures_iter(template_src) {
        if let Some(var_match) = caps.get(1) {
            let path = var_match.as_str();
            let mut segments = path.split('.');
            let root = segments.next().unwrap_or_default();
            match root {
                "issue" => {
                    if let Some(field) = segments.next() {
                        if let Some(map) = allowed_issue_fields {
                            if !map.contains_key(field) {
                                return Err(SymphonyError::TemplateRenderError(format!(
                                    "unknown issue field 'issue.{field}'"
                                )));
                            }
                        }
                    }
                }
                "attempt" => {}
                other => {
                    return Err(SymphonyError::TemplateRenderError(format!(
                        "unknown template variable '{other}'"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn json_to_liquid(val: &Value) -> LiquidValue {
    match val {
        Value::Null => LiquidValue::Nil,
        Value::Bool(b) => LiquidValue::scalar(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                LiquidValue::scalar(i)
            } else if let Some(f) = n.as_f64() {
                LiquidValue::scalar(f)
            } else {
                LiquidValue::Nil
            }
        }
        Value::String(s) => LiquidValue::scalar(s.clone()),
        Value::Array(arr) => LiquidValue::Array(arr.iter().map(json_to_liquid).collect()),
        Value::Object(map) => {
            let mut obj = liquid::Object::new();
            for (k, v) in map {
                obj.insert(k.clone().into(), json_to_liquid(v));
            }
            LiquidValue::Object(obj)
        }
    }
}

/// Build continuation turn guidance (SPEC.md Section 7.1 & Elixir reference prompt_builder)
pub fn render_continuation_prompt(issue: &Issue, turn_number: u32, max_turns: u32) -> String {
    format!(
        "Continuation guidance:\n\n\
        - The previous Codex turn completed normally, but the issue `{}` ({}) is still in an active state (`{}`).\n\
        - This is continuation turn #{} of {}.\n\
        - Resume from the current workspace and workpad state instead of restarting from scratch.\n\
        - The original task instructions and prior turn context are already present in this thread, so do not restate them before acting.\n\
        - Focus on the remaining ticket work and validation checks before ending the turn.",
        issue.identifier, issue.title, issue.state, turn_number, max_turns
    )
}

/// Compute retry backoff delay in milliseconds (SPEC.md Section 8.4)
/// - Normal continuation retries use fixed 1000ms.
/// - Failure retries use `min(10000 * 2^(attempt - 1), max_retry_backoff_ms)`.
pub fn compute_backoff_delay_ms(
    attempt: u32,
    is_normal_continuation: bool,
    max_retry_backoff_ms: u64,
) -> u64 {
    if is_normal_continuation {
        return 1_000;
    }
    let exp = attempt.saturating_sub(1).min(20);
    let raw = 10_000u64.saturating_mul(1u64 << exp);
    raw.min(max_retry_backoff_ms)
}
