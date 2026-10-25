use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use symphony_core::{
    BlockerRef, ChangedFileSummary, Issue, ReviewCommentEvaluation, SymphonyError, SymphonyResult,
};
use tracing::warn;

pub mod tool_policy;

/// Pages read per state listing (startup cleanup of terminal issues) before giving up on the rest,
/// so the closed history of an old repository cannot stall startup.
const MAX_ISSUE_PAGES: usize = 10;
/// Pages read per candidate listing. Open work has to be seen completely (GitHub lists oldest
/// first, so a low cap would hide every newer issue); this only bounds a runaway listing.
const MAX_CANDIDATE_PAGES: usize = 100;
const GITHUB_PAGE_SIZE: usize = 100;
const LINEAR_PAGE_SIZE: usize = 50;

#[async_trait]
pub trait TrackerAdapter: Send + Sync {
    fn kind(&self) -> &'static str;

    /// Return issues in configured active states matching required labels (SPEC.md Section 11.1)
    async fn fetch_candidate_issues(
        &self,
        active_states: &[String],
        required_labels: &[String],
    ) -> SymphonyResult<Vec<Issue>>;

    /// Return issues in the specified states (used for startup terminal workspace cleanup)
    async fn fetch_issues_by_states(&self, state_names: &[String]) -> SymphonyResult<Vec<Issue>>;

    /// Return refreshed issue snapshots for currently running/claimed issue IDs (Reconciliation Part B)
    async fn fetch_issue_states_by_ids(&self, issue_ids: &[String]) -> SymphonyResult<Vec<Issue>>;

    /// Post a comment on an issue/ticket through host-controlled credentials
    async fn create_comment(&self, issue_id: &str, body: &str) -> SymphonyResult<()>;

    /// Transition an issue/ticket state
    async fn update_issue_state(&self, issue_id: &str, state_name: &str) -> SymphonyResult<()>;
}

// =============================================================================
// GitHub Issues + Pull Request + CI Adapter
// =============================================================================

#[derive(Clone)]
pub struct GitHubTracker {
    client: Client,
    /// Used for the agent-facing tool: never follows a redirect, so a redirecting answer cannot
    /// take the host's credentials to another host.
    tool_client: Client,
    api_url: String,
    repo: String,
    token: Option<String>,
    assignee: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPullRequestDetails {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub html_url: String,
    pub state: String,
    pub head_ref: String,
    /// The commit the pull request's head branch points to.
    pub head_sha: String,
    pub base_ref: String,
    /// `None` while GitHub has not computed mergeability yet (`mergeable: null`): unknown, not "yes".
    pub mergeable: Option<bool>,
    pub has_conflicts: bool,
    /// `passed`, `failed`, `pending`, `none` (no check ran) or `unknown` (the checks could not be read).
    pub ci_status: String,
    /// The pull request was merged (by someone; Symphony never merges).
    pub merged: bool,
    pub changed_files: Vec<ChangedFileSummary>,
    pub review_comments: Vec<ReviewCommentEvaluation>,
}

/// A pull request opened by (or found for) a branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedPullRequest {
    pub number: u64,
    pub html_url: String,
    /// `true` when the branch already had an open pull request and none was created.
    pub already_existed: bool,
}

impl GitHubTracker {
    pub fn new(
        api_url: Option<String>,
        repo: String,
        token: Option<String>,
        assignee: Option<String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("symphony-desktop-orchestrator/1.0")
            .build()
            .unwrap_or_else(|_| Client::new());

        let tool_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("symphony-desktop-orchestrator/1.0")
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            tool_client,
            api_url: api_url
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "https://api.github.com".to_string())
                .trim_end_matches('/')
                .to_string(),
            repo: repo.trim().trim_matches('/').to_string(),
            token: token.filter(|t| !t.trim().is_empty()),
            assignee: assignee.filter(|a| !a.trim().is_empty()),
        }
    }

    fn validate_config(&self) -> SymphonyResult<()> {
        if self.repo.is_empty() || !self.repo.contains('/') {
            return Err(SymphonyError::TrackerError {
                code: "missing_github_repo".to_string(),
                message: "GitHub tracker requires 'owner/repo' repository slug".to_string(),
            });
        }
        Ok(())
    }

    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut r = req
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(tok) = &self.token {
            r = r.bearer_auth(tok);
        }
        r
    }

    pub fn normalize_issue(&self, raw: &Value) -> Option<Issue> {
        let obj = raw.as_object()?;
        let number = obj.get("number")?.as_u64()?;
        let title = obj.get("title")?.as_str()?.to_string();
        let state = obj.get("state")?.as_str()?.to_string();
        let is_pr = obj.contains_key("pull_request");

        let description = obj
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let url = obj
            .get("html_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let assignee_id = obj
            .get("assignee")
            .and_then(|a| a.get("login"))
            .and_then(|l| l.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                obj.get("assignees")
                    .and_then(|arr| arr.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|a| a.get("login"))
                    .and_then(|l| l.as_str())
                    .map(|s| s.to_string())
            });

        let labels: Vec<String> = obj
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item.as_str().map(|s| s.trim().to_lowercase()).or_else(|| {
                            item.get("name")
                                .and_then(|n| n.as_str())
                                .map(|s| s.trim().to_lowercase())
                        })
                    })
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Derive priority from labels if present (e.g., p1/priority:high -> 1, p2 -> 2, p3 -> 3, p4 -> 4)
        let priority = derive_priority_from_labels(&labels);

        let created_at = obj
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(parse_iso_datetime);
        let updated_at = obj
            .get("updated_at")
            .and_then(|v| v.as_str())
            .and_then(parse_iso_datetime);

        let assigned_to_worker = match &self.assignee {
            None => true,
            Some(expected) => assignee_id
                .as_ref()
                .map(|actual| actual.eq_ignore_ascii_case(expected))
                .unwrap_or(false),
        };

        Some(Issue {
            id: number.to_string(),
            native_ref: Some(json!({
                "repository": self.repo,
                "issue_number": number
            })),
            identifier: format!("GH-{number}"),
            title,
            description,
            priority,
            state,
            branch_name: None,
            url,
            assignee_id,
            labels,
            blocked_by: Vec::new(),
            dispatchable: !is_pr && assigned_to_worker,
            created_at,
            updated_at,
        })
    }

    pub fn dynamic_tool_spec() -> Value {
        json!({
            "name": "github_api",
            "description": "Execute an authenticated GitHub REST API request on the host side without exposing GITHUB_TOKEN to the agent subprocess. Limited to the project's own repository (paths under /repos/{owner}/{repo}/): read it, open and edit issues, edit pull requests and comment on issues and pull requests. Opening a pull request, merging, deleting, administration and other repositories are not available: a pull request is created only by the human-confirmed \"Create pull request\" action.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "method": { "type": "string", "description": "HTTP method (GET, POST or PATCH)" },
                    "path": { "type": "string", "description": "Relative GitHub REST API path under /repos/{owner}/{repo}/" },
                    "body": { "type": "object", "description": "Optional JSON request body" }
                },
                "required": ["method", "path"]
            }
        })
    }

    pub async fn execute_github_dynamic_tool(&self, arguments: Value) -> (bool, String) {
        let method = arguments
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET");
        let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let body = arguments.get("body");
        // The agent reads untrusted text: what it may ask the host to do with Symphony's own
        // credentials is limited by `tool_policy`, before anything is sent.
        if let Err(reason) = tool_policy::check_github_request(&self.repo, method, path) {
            return (false, json!({ "error": { "message": reason } }).to_string());
        }
        match self.execute_dynamic_tool(method, path, body).await {
            Ok((ok, val)) => (ok, serde_json::to_string_pretty(&val).unwrap_or_default()),
            Err(e) => (false, json!({ "error": e.to_string() }).to_string()),
        }
    }

    /// Host-side execution of the `github_api` dynamic tool for Codex App Server
    pub async fn execute_dynamic_tool(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> SymphonyResult<(bool, Value)> {
        self.validate_config()?;
        if !path.starts_with('/') || path.starts_with("//") || path.contains("://") {
            return Ok((
                false,
                json!({"error": {"message": "github_api path must be a relative API path starting with '/'"}}),
            ));
        }

        let url = format!("{}{}", self.api_url, path);
        let req_method = reqwest::Method::from_bytes(method.trim().to_uppercase().as_bytes())
            .map_err(|e| SymphonyError::TrackerError {
                code: "invalid_method".to_string(),
                message: e.to_string(),
            })?;

        let mut req = self.apply_headers(self.tool_client.request(req_method, &url));
        if let Some(b) = body {
            req = req.json(b);
        }

        let resp = req.send().await.map_err(|e| SymphonyError::TrackerError {
            code: "github_api_request_failed".to_string(),
            message: e.to_string(),
        })?;

        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .unwrap_or_else(|_| json!({"status": status.as_u16()}));
        Ok((status.is_success(), val))
    }

    /// Open a pull request for `head` into `base`, or return the open one the branch already has
    /// (a retry after a pull request was created but not recorded must not fail or duplicate it).
    /// Nothing is ever merged.
    pub async fn open_pull_request(
        &self,
        title: &str,
        head: &str,
        base: &str,
        body: &str,
    ) -> SymphonyResult<OpenedPullRequest> {
        self.validate_config()?;
        let url = format!("{}/repos/{}/pulls", self.api_url, self.repo);
        let resp = self
            .apply_headers(self.client.post(&url))
            .json(&json!({ "title": title, "head": head, "base": base, "body": body }))
            .send()
            .await
            .map_err(|e| SymphonyError::TrackerError {
                code: "github_pr_create_failed".to_string(),
                message: e.to_string(),
            })?;
        let status = resp.status();
        if status.is_success() {
            let created: Value = resp.json().await.map_err(|e| SymphonyError::TrackerError {
                code: "github_pr_decode_failed".to_string(),
                message: e.to_string(),
            })?;
            return Self::opened_from_json(&created, false).ok_or_else(|| {
                SymphonyError::TrackerError {
                    code: "github_pr_decode_failed".to_string(),
                    message: "GitHub's answer has no pull request number".to_string(),
                }
            });
        }
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 422 && text.contains("already exists") {
            // Only a pull request into the requested base is the one asked for.
            if let Some(existing) = self.find_open_pull_request(head, base).await? {
                return Ok(existing);
            }
        }
        Err(SymphonyError::TrackerError {
            code: format!("github_http_{}", status.as_u16()),
            message: text,
        })
    }

    /// The open pull request from branch `head` of this repository into `base`, if any.
    pub async fn find_open_pull_request(
        &self,
        head: &str,
        base: &str,
    ) -> SymphonyResult<Option<OpenedPullRequest>> {
        self.validate_config()?;
        let owner = self.repo.split('/').next().unwrap_or_default();
        let mut url = url::Url::parse(&format!("{}/repos/{}/pulls", self.api_url, self.repo))
            .map_err(|e| SymphonyError::TrackerError {
                code: "github_pr_fetch_failed".to_string(),
                message: e.to_string(),
            })?;
        url.query_pairs_mut()
            .append_pair("head", &format!("{owner}:{head}"))
            .append_pair("base", base)
            .append_pair("state", "open");
        let resp = self
            .apply_headers(self.client.get(url))
            .send()
            .await
            .map_err(|e| SymphonyError::TrackerError {
                code: "github_pr_fetch_failed".to_string(),
                message: e.to_string(),
            })?;
        if !resp.status().is_success() {
            return Err(SymphonyError::TrackerError {
                code: format!("github_http_{}", resp.status().as_u16()),
                message: resp.text().await.unwrap_or_default(),
            });
        }
        let list: Value = resp.json().await.unwrap_or(Value::Null);
        Ok(list
            .as_array()
            .and_then(|items| items.first())
            .and_then(|first| Self::opened_from_json(first, true)))
    }

    fn opened_from_json(value: &Value, already_existed: bool) -> Option<OpenedPullRequest> {
        Some(OpenedPullRequest {
            number: value.get("number")?.as_u64()?,
            html_url: value
                .get("html_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            already_existed,
        })
    }

    /// Create a Pull Request on GitHub
    pub async fn create_pull_request(
        &self,
        title: &str,
        head: &str,
        base: &str,
        body: &str,
    ) -> SymphonyResult<GitHubPullRequestDetails> {
        self.validate_config()?;
        let url = format!("{}/repos/{}/pulls", self.api_url, self.repo);
        let payload = json!({
            "title": title,
            "head": head,
            "base": base,
            "body": body,
        });

        let resp = self
            .apply_headers(self.client.post(&url))
            .json(&payload)
            .send()
            .await
            .map_err(|e| SymphonyError::TrackerError {
                code: "github_pr_create_failed".to_string(),
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(SymphonyError::TrackerError {
                code: format!("github_http_{status}"),
                message: err_text,
            });
        }

        let pr_json: Value = resp.json().await.map_err(|e| SymphonyError::TrackerError {
            code: "github_pr_decode_failed".to_string(),
            message: e.to_string(),
        })?;
        let pr_number = pr_json.get("number").and_then(|n| n.as_u64()).unwrap_or(1);
        self.fetch_pull_request_details(pr_number).await
    }

    /// Fetch full Pull Request details, CI status, changed files, and review comments (evaluating agent vs human comments)
    pub async fn fetch_pull_request_details(
        &self,
        pr_number: u64,
    ) -> SymphonyResult<GitHubPullRequestDetails> {
        self.validate_config()?;
        let pr_url = format!("{}/repos/{}/pulls/{pr_number}", self.api_url, self.repo);
        let resp = self
            .apply_headers(self.client.get(&pr_url))
            .send()
            .await
            .map_err(|e| SymphonyError::TrackerError {
                code: "github_pr_fetch_failed".to_string(),
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(SymphonyError::TrackerError {
                code: format!("github_http_{}", resp.status().as_u16()),
                message: format!("Failed to fetch PR #{pr_number}"),
            });
        }

        let pr: Value = resp.json().await.map_err(|e| SymphonyError::TrackerError {
            code: "github_pr_decode_failed".to_string(),
            message: e.to_string(),
        })?;

        let title = pr
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let body = pr
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let html_url = pr
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let state = pr
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("open")
            .to_string();
        let head_ref = pr
            .get("head")
            .and_then(|h| h.get("ref"))
            .and_then(|v| v.as_str())
            .unwrap_or("HEAD")
            .to_string();
        let head_sha = pr
            .get("head")
            .and_then(|h| h.get("sha"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let base_ref = pr
            .get("base")
            .and_then(|b| b.get("ref"))
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string();
        // GitHub answers `mergeable: null` while it computes the merge: that is unknown, and an
        // unknown merge is never reported as clean.
        let mergeable = pr.get("mergeable").and_then(|v| v.as_bool());
        let mergeable_state = pr
            .get("mergeable_state")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let has_conflicts = mergeable == Some(false) || mergeable_state == "dirty";
        let merged = pr.get("merged").and_then(|v| v.as_bool()).unwrap_or(false)
            || pr.get("merged_at").map(|v| !v.is_null()).unwrap_or(false);

        // Fetch changed files
        let files_url = format!(
            "{}/repos/{}/pulls/{pr_number}/files",
            self.api_url, self.repo
        );
        let mut changed_files = Vec::new();
        if let Ok(f_resp) = self.apply_headers(self.client.get(&files_url)).send().await {
            if let Ok(Value::Array(arr)) = f_resp.json::<Value>().await {
                for item in arr {
                    if let Some(path) = item.get("filename").and_then(|v| v.as_str()) {
                        changed_files.push(ChangedFileSummary {
                            path: path.to_string(),
                            status: item
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("modified")
                                .to_string(),
                            additions: item.get("additions").and_then(|v| v.as_u64()).unwrap_or(0)
                                as u32,
                            deletions: item.get("deletions").and_then(|v| v.as_u64()).unwrap_or(0)
                                as u32,
                        });
                    }
                }
            }
        }

        // Fetch CI check runs for head_sha
        let mut ci_status = "unknown".to_string();
        if !head_sha.is_empty() {
            let checks_url = format!(
                "{}/repos/{}/commits/{head_sha}/check-runs?per_page=100",
                self.api_url, self.repo
            );
            if let Ok(c_resp) = self
                .apply_headers(self.client.get(&checks_url))
                .send()
                .await
            {
                if let Ok(val) = c_resp.json::<Value>().await {
                    if let Some(runs) = val.get("check_runs").and_then(|r| r.as_array()) {
                        let total = val
                            .get("total_count")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(runs.len() as u64);
                        // More runs than were read: a failure may sit on a page that was not
                        // read, so the summary is only trustworthy for what is a failure already.
                        ci_status = match summarize_check_runs(runs) {
                            "passed" | "pending" | "none" if total > runs.len() as u64 => "unknown",
                            other => other,
                        }
                        .to_string();
                    }
                }
            }
            // CI systems that report commit statuses instead of check runs (Jenkins, Buildkite,
            // older integrations): a red status must not hide behind green or absent check runs.
            let status_url = format!(
                "{}/repos/{}/commits/{head_sha}/status?per_page=100",
                self.api_url, self.repo
            );
            let statuses = match self
                .apply_headers(self.client.get(&status_url))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
                    Ok(combined) => summarize_commit_statuses(&combined),
                    Err(_) => "unknown",
                },
                _ => "unknown",
            };
            ci_status = combine_ci_signals(&ci_status, statuses).to_string();
        }

        // Fetch review comments and evaluate agent comments against actual changed files
        let comments_url = format!(
            "{}/repos/{}/pulls/{pr_number}/comments",
            self.api_url, self.repo
        );
        let mut review_comments = Vec::new();
        if let Ok(cm_resp) = self
            .apply_headers(self.client.get(&comments_url))
            .send()
            .await
        {
            if let Ok(Value::Array(arr)) = cm_resp.json::<Value>().await {
                for item in arr {
                    let author = item
                        .get("user")
                        .and_then(|u| u.get("login"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("reviewer")
                        .to_string();
                    let user_type = item
                        .get("user")
                        .and_then(|u| u.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("User");
                    let is_agent = user_type.eq_ignore_ascii_case("Bot")
                        || author.contains("[bot]")
                        || author.contains("copilot")
                        || author.contains("codex");
                    let comment_body = item
                        .get("body")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let file_path = item
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let verified_against_diff = match &file_path {
                        Some(fp) => changed_files.iter().any(|cf| &cf.path == fp),
                        None => !changed_files.is_empty(),
                    };

                    let evaluation_summary = if is_agent {
                        if verified_against_diff {
                            "Agent review comment targets an active file in the PR diff; verify line context before acting.".to_string()
                        } else {
                            "Agent review comment references a path not modified in the current diff; treat with caution.".to_string()
                        }
                    } else {
                        "Human reviewer feedback.".to_string()
                    };

                    review_comments.push(ReviewCommentEvaluation {
                        author,
                        is_agent,
                        body: comment_body,
                        file_path,
                        verified_against_diff,
                        evaluation_summary,
                    });
                }
            }
        }

        Ok(GitHubPullRequestDetails {
            number: pr_number,
            title,
            body,
            html_url,
            state,
            head_ref,
            head_sha,
            base_ref,
            mergeable,
            has_conflicts,
            ci_status,
            merged,
            changed_files,
            review_comments,
        })
    }
}

/// CI status of a commit from its check runs. No check run means no CI ran: that is `none`, never
/// `passed`; a cancelled, timed-out or action-required run is not a pass either.
/// The combined commit status (`GET /commits/{sha}/status`): GitHub's own roll-up of the latest
/// status per context.
fn summarize_commit_statuses(combined: &Value) -> &'static str {
    let count = combined
        .get("statuses")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let total = combined
        .get("total_count")
        .and_then(Value::as_u64)
        .unwrap_or(count as u64);
    if count == 0 && total == 0 {
        return "none";
    }
    match combined.get("state").and_then(Value::as_str) {
        Some("failure" | "error") => "failed",
        Some("pending") => "pending",
        Some("success") => "passed",
        _ => "unknown",
    }
}

/// One CI verdict from check runs and commit statuses: any failure fails; what could not be read
/// is unknown (never a pass); otherwise pending wins over passed, and two absences are "none".
fn combine_ci_signals(check_runs: &str, statuses: &str) -> &'static str {
    let either = |value: &str| check_runs == value || statuses == value;
    if either("failed") {
        "failed"
    } else if either("unknown") {
        "unknown"
    } else if either("pending") {
        "pending"
    } else if either("passed") {
        "passed"
    } else {
        "none"
    }
}

fn summarize_check_runs(runs: &[Value]) -> &'static str {
    if runs.is_empty() {
        return "none";
    }
    let conclusion = |r: &Value| {
        r.get("conclusion")
            .and_then(|c| c.as_str())
            .map(str::to_string)
    };
    if runs.iter().any(|r| {
        matches!(
            conclusion(r).as_deref(),
            Some(
                "failure"
                    | "timed_out"
                    | "cancelled"
                    | "action_required"
                    | "startup_failure"
                    | "stale"
            )
        )
    }) {
        return "failed";
    }
    if runs
        .iter()
        .any(|r| r.get("status").and_then(|s| s.as_str()) != Some("completed"))
    {
        return "pending";
    }
    if runs.iter().all(|r| {
        matches!(
            conclusion(r).as_deref(),
            Some("success" | "neutral" | "skipped")
        )
    }) {
        return "passed";
    }
    "unknown"
}

impl GitHubTracker {
    /// Issues in `state_names`, reading at most `max_pages` pages.
    async fn list_issues(
        &self,
        state_names: &[String],
        max_pages: usize,
    ) -> SymphonyResult<Vec<Issue>> {
        self.validate_config()?;
        let normalized_states: Vec<String> = state_names
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if normalized_states.is_empty() {
            return Ok(Vec::new());
        }

        // Map workflow states (like "Todo", "In Progress", "open") to GitHub API `open` / `closed`
        let wants_open = normalized_states
            .iter()
            .any(|s| s == "open" || s == "todo" || s == "in progress" || s == "ready");
        let wants_closed = normalized_states
            .iter()
            .any(|s| s == "closed" || s == "done" || s == "completed" || s == "cancelled");

        let api_state = match (wants_open, wants_closed) {
            (true, true) => "all",
            (false, true) => "closed",
            _ => "open",
        };

        // Candidates oldest first (SPEC.md Section 8.2); a closed-only listing (terminal cleanup)
        // most recently updated first: closing an issue updates it, and recently closed issues
        // are the ones that may still have workspaces.
        let order = if api_state == "closed" {
            "sort=updated&direction=desc"
        } else {
            "sort=created&direction=asc"
        };
        let mut out = Vec::new();
        for page in 1..=max_pages {
            let url = format!(
                "{}/repos/{}/issues?state={api_state}&per_page={GITHUB_PAGE_SIZE}&page={page}&{order}",
                self.api_url, self.repo
            );

            let resp = self
                .apply_headers(self.client.get(&url))
                .send()
                .await
                .map_err(|e| SymphonyError::TrackerError {
                    code: "github_api_request".to_string(),
                    message: e.to_string(),
                })?;

            if !resp.status().is_success() {
                return Err(SymphonyError::TrackerError {
                    code: format!("github_http_{}", resp.status().as_u16()),
                    message: resp.text().await.unwrap_or_default(),
                });
            }

            let items: Vec<Value> = resp.json().await.map_err(|e| SymphonyError::TrackerError {
                code: "github_payload_error".to_string(),
                message: e.to_string(),
            })?;

            for item in &items {
                if let Some(issue) = self.normalize_issue(item) {
                    if !issue.id.is_empty() {
                        out.push(issue);
                    }
                }
            }
            // A short page is the last one (pull requests count towards the page size too).
            if items.len() < GITHUB_PAGE_SIZE {
                return Ok(out);
            }
        }
        warn!(
            repo = %self.repo,
            pages = max_pages,
            "GitHub issue listing truncated; only the first pages are considered"
        );
        Ok(out)
    }
}

#[async_trait]
impl TrackerAdapter for GitHubTracker {
    fn kind(&self) -> &'static str {
        "github"
    }

    async fn fetch_candidate_issues(
        &self,
        active_states: &[String],
        required_labels: &[String],
    ) -> SymphonyResult<Vec<Issue>> {
        let issues = self.list_issues(active_states, MAX_CANDIDATE_PAGES).await?;
        Ok(issues
            .into_iter()
            .filter(|i| i.dispatchable && i.has_required_labels(required_labels))
            .collect())
    }

    async fn fetch_issues_by_states(&self, state_names: &[String]) -> SymphonyResult<Vec<Issue>> {
        self.list_issues(state_names, MAX_ISSUE_PAGES).await
    }

    async fn fetch_issue_states_by_ids(&self, issue_ids: &[String]) -> SymphonyResult<Vec<Issue>> {
        self.validate_config()?;
        let mut out = Vec::new();
        for id in issue_ids {
            let trimmed = id.trim().trim_start_matches("GH-");
            if trimmed.is_empty() {
                continue;
            }
            let url = format!("{}/repos/{}/issues/{trimmed}", self.api_url, self.repo);
            let resp = self
                .apply_headers(self.client.get(&url))
                .send()
                .await
                .map_err(|e| SymphonyError::TrackerError {
                    code: "github_api_request".to_string(),
                    message: e.to_string(),
                })?;

            if resp.status().as_u16() == 404 {
                continue;
            }
            if !resp.status().is_success() {
                return Err(SymphonyError::TrackerError {
                    code: format!("github_http_{}", resp.status().as_u16()),
                    message: resp.text().await.unwrap_or_default(),
                });
            }
            let val: Value = resp.json().await.map_err(|e| SymphonyError::TrackerError {
                code: "github_payload_error".to_string(),
                message: e.to_string(),
            })?;
            if let Some(issue) = self.normalize_issue(&val) {
                out.push(issue);
            }
        }
        Ok(out)
    }

    async fn create_comment(&self, issue_id: &str, body: &str) -> SymphonyResult<()> {
        self.validate_config()?;
        let num = issue_id.trim().trim_start_matches("GH-");
        let url = format!("{}/repos/{}/issues/{num}/comments", self.api_url, self.repo);
        let resp = self
            .apply_headers(self.client.post(&url))
            .json(&json!({ "body": body }))
            .send()
            .await
            .map_err(|e| SymphonyError::TrackerError {
                code: "github_comment_failed".to_string(),
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(SymphonyError::TrackerError {
                code: format!("github_http_{}", resp.status().as_u16()),
                message: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(())
    }

    async fn update_issue_state(&self, issue_id: &str, state_name: &str) -> SymphonyResult<()> {
        self.validate_config()?;
        let num = issue_id.trim().trim_start_matches("GH-");
        let gh_state = match state_name.trim().to_lowercase().as_str() {
            "closed" | "done" | "completed" | "cancelled" => "closed",
            _ => "open",
        };
        let url = format!("{}/repos/{}/issues/{num}", self.api_url, self.repo);
        let resp = self
            .apply_headers(self.client.patch(&url))
            .json(&json!({ "state": gh_state }))
            .send()
            .await
            .map_err(|e| SymphonyError::TrackerError {
                code: "github_state_update_failed".to_string(),
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(SymphonyError::TrackerError {
                code: format!("github_http_{}", resp.status().as_u16()),
                message: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(())
    }
}

// =============================================================================
// Linear GraphQL Adapter (SPEC.md Section 11 & Elixir linear/client.ex)
// =============================================================================

const LINEAR_COLLECTION_QUERY: &str = r#"
query SymphonyLinearPoll($projectSlug: String!, $stateNames: [String!]!, $first: Int!, $relationFirst: Int!, $after: String) {
  issues(filter: {project: {slugId: {eq: $projectSlug}}, state: {name: {in: $stateNames}}}, first: $first, after: $after) {
    nodes {
      id
      identifier
      title
      description
      priority
      state {
        name
      }
      branchName
      url
      assignee {
        id
      }
      labels {
        nodes {
          name
        }
      }
      inverseRelations(first: $relationFirst) {
        nodes {
          type
          issue {
            id
            identifier
            state {
              name
            }
          }
        }
      }
      createdAt
      updatedAt
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;

const LINEAR_BY_IDS_QUERY: &str = r#"
query SymphonyLinearIssuesById($ids: [ID!]!, $first: Int!, $relationFirst: Int!) {
  issues(filter: {id: {in: $ids}}, first: $first) {
    nodes {
      id
      identifier
      title
      description
      priority
      state {
        name
      }
      branchName
      url
      assignee {
        id
      }
      labels {
        nodes {
          name
        }
      }
      inverseRelations(first: $relationFirst) {
        nodes {
          type
          issue {
            id
            identifier
            state {
              name
            }
          }
        }
      }
      createdAt
      updatedAt
    }
  }
}
"#;

#[derive(Clone)]
pub struct LinearTracker {
    client: Client,
    endpoint: String,
    project_slug: String,
    api_key: Option<String>,
    terminal_states: Vec<String>,
}

impl LinearTracker {
    pub fn new(
        endpoint: Option<String>,
        project_slug: String,
        api_key: Option<String>,
        terminal_states: Vec<String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("symphony-desktop-orchestrator/1.0")
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            endpoint: endpoint
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "https://api.linear.app/graphql".to_string()),
            project_slug: project_slug.trim().to_string(),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            terminal_states,
        }
    }

    pub fn dynamic_tool_spec() -> Value {
        json!({
            "name": "linear_graphql",
            "description": "Execute a single GraphQL operation against Linear using Symphony's host-side authenticated session. Queries are read-only; mutations are limited to commentCreate, commentUpdate, issueUpdate, attachmentCreate, attachmentLinkGitHubPR and attachmentLinkURL, on issues (and their comments) of the configured project only; name the target with an `id`/`issueId` string or variable. Issues cannot be moved to another project or team.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "GraphQL query or mutation string" },
                    "variables": { "type": "object", "description": "Optional GraphQL variables map" }
                },
                "required": ["query"]
            }
        })
    }

    pub async fn execute_linear_dynamic_tool(&self, arguments: Value) -> (bool, String) {
        let query = match arguments.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q,
            _ => {
                return (
                    false,
                    json!({ "error": { "message": "linear_graphql requires a non-empty `query` string" } }).to_string(),
                )
            }
        };
        if let Err(reason) = tool_policy::check_linear_document(query) {
            return (false, json!({ "error": { "message": reason } }).to_string());
        }
        let variables = arguments
            .get("variables")
            .cloned()
            .unwrap_or_else(|| json!({}));
        // Mutations only change issues (and their comments) of the configured project; the key
        // itself is workspace-wide.
        match tool_policy::linear_mutation_targets(query, &variables) {
            Err(reason) => return (false, json!({ "error": { "message": reason } }).to_string()),
            Ok(Some(targets)) => {
                if let Err(reason) = self.check_linear_targets_in_project(&targets).await {
                    return (false, json!({ "error": { "message": reason } }).to_string());
                }
            }
            Ok(None) => {}
        }
        match self.graphql(query, variables).await {
            Ok(val) => (true, serde_json::to_string_pretty(&val).unwrap_or_default()),
            Err(e) => (false, json!({ "error": e.to_string() }).to_string()),
        }
    }

    /// The configured project's `slugId` of an issue (by id or identifier) or of a comment's issue;
    /// `None` when it is neither, or has no project.
    async fn linear_target_project(&self, id: &str) -> Option<String> {
        const ISSUE: &str =
            "query SymphonyScopeIssue($id: String!) { issue(id: $id) { project { slugId } } }";
        const COMMENT: &str = "query SymphonyScopeComment($id: String!) { comment(id: $id) { issue { project { slugId } } } }";
        if let Ok(body) = self.graphql(ISSUE, json!({ "id": id })).await {
            if let Some(slug) = body
                .pointer("/data/issue/project/slugId")
                .and_then(Value::as_str)
            {
                return Some(slug.to_string());
            }
        }
        let body = self.graphql(COMMENT, json!({ "id": id })).await.ok()?;
        body.pointer("/data/comment/issue/project/slugId")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    async fn check_linear_targets_in_project(&self, targets: &[String]) -> Result<(), String> {
        if self.project_slug.is_empty() {
            return Err("linear_graphql mutations need a configured Linear project".into());
        }
        for target in targets {
            match self.linear_target_project(target).await {
                Some(slug) if slug == self.project_slug => {}
                _ => {
                    return Err(format!(
                        "linear_graphql may only change issues of the configured project; `{target}` is not one of them"
                    ))
                }
            }
        }
        Ok(())
    }

    pub async fn graphql(&self, query: &str, variables: Value) -> SymphonyResult<Value> {
        let token = self
            .api_key
            .as_ref()
            .ok_or_else(|| SymphonyError::TrackerError {
                code: "missing_linear_api_token".to_string(),
                message: "Linear API key is required".to_string(),
            })?;

        let resp = self
            .client
            .post(&self.endpoint)
            .header("Authorization", token)
            .header("Content-Type", "application/json")
            .json(&json!({
                "query": query,
                "variables": variables,
            }))
            .send()
            .await
            .map_err(|e| SymphonyError::TrackerError {
                code: "linear_api_request".to_string(),
                message: e.to_string(),
            })?;

        if !resp.status().is_success() {
            return Err(SymphonyError::TrackerError {
                code: "linear_api_status".to_string(),
                message: format!("HTTP {}", resp.status().as_u16()),
            });
        }

        let body: Value = resp.json().await.map_err(|e| SymphonyError::TrackerError {
            code: "linear_payload_error".to_string(),
            message: e.to_string(),
        })?;

        if let Some(errors) = body.get("errors") {
            return Err(SymphonyError::TrackerError {
                code: "linear_graphql_errors".to_string(),
                message: errors.to_string(),
            });
        }

        Ok(body)
    }

    pub fn normalize_issue(&self, raw: &Value) -> Option<Issue> {
        let obj = raw.as_object()?;
        let id = obj.get("id")?.as_str()?.to_string();
        let identifier = obj.get("identifier")?.as_str()?.to_string();
        let title = obj.get("title")?.as_str()?.to_string();
        let description = obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let priority = obj
            .get("priority")
            .and_then(|v| v.as_i64())
            .map(|p| p as i32);
        let state = obj
            .get("state")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str())?
            .to_string();
        let branch_name = obj
            .get("branchName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let url = obj
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let assignee_id = obj
            .get("assignee")
            .and_then(|a| a.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let labels: Vec<String> = obj
            .get("labels")
            .and_then(|l| l.get("nodes"))
            .and_then(|n| n.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("name").and_then(|n| n.as_str()))
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let blocked_by: Vec<BlockerRef> = obj
            .get("inverseRelations")
            .and_then(|r| r.get("nodes"))
            .and_then(|n| n.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|rel| {
                        rel.get("type")
                            .and_then(|t| t.as_str())
                            .map(|t| t.trim().eq_ignore_ascii_case("blocks"))
                            .unwrap_or(false)
                    })
                    .filter_map(|rel| {
                        let blocker = rel.get("issue")?;
                        Some(BlockerRef {
                            id: blocker
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            identifier: blocker
                                .get("identifier")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            state: blocker
                                .get("state")
                                .and_then(|s| s.get("name"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let is_todo_with_non_terminal_blocker = state.trim().eq_ignore_ascii_case("todo")
            && blocked_by.iter().any(|b| {
                let bs = b.state.as_deref().unwrap_or("").trim().to_lowercase();
                !self
                    .terminal_states
                    .iter()
                    .any(|ts| ts.trim().to_lowercase() == bs)
            });

        let created_at = obj
            .get("createdAt")
            .and_then(|v| v.as_str())
            .and_then(parse_iso_datetime);
        let updated_at = obj
            .get("updatedAt")
            .and_then(|v| v.as_str())
            .and_then(parse_iso_datetime);

        Some(Issue {
            id: id.clone(),
            native_ref: Some(json!({ "id": id })),
            identifier,
            title,
            description,
            priority,
            state,
            branch_name,
            url,
            assignee_id,
            labels,
            blocked_by,
            dispatchable: !is_todo_with_non_terminal_blocker,
            created_at,
            updated_at,
        })
    }
}

impl LinearTracker {
    /// Issues in `state_names`, reading at most `max_pages` pages.
    async fn list_issues(
        &self,
        state_names: &[String],
        max_pages: usize,
    ) -> SymphonyResult<Vec<Issue>> {
        if self.project_slug.is_empty() {
            return Err(SymphonyError::TrackerError {
                code: "missing_linear_project_slug".to_string(),
                message: "Linear project_slug is required".to_string(),
            });
        }
        if state_names.is_empty() {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        let mut after = Value::Null;
        for page in 1..=max_pages {
            let body = self
                .graphql(
                    LINEAR_COLLECTION_QUERY,
                    json!({
                        "projectSlug": self.project_slug,
                        "stateNames": state_names,
                        "first": LINEAR_PAGE_SIZE,
                        "relationFirst": 50,
                        "after": after
                    }),
                )
                .await?;

            let issues = body.pointer("/data/issues");
            let nodes = issues
                .and_then(|i| i.get("nodes"))
                .and_then(|n| n.as_array())
                .cloned()
                .unwrap_or_default();
            out.extend(nodes.iter().filter_map(|n| self.normalize_issue(n)));

            let has_next = issues
                .and_then(|i| i.pointer("/pageInfo/hasNextPage"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let cursor = issues
                .and_then(|i| i.pointer("/pageInfo/endCursor"))
                .and_then(|v| v.as_str());
            if !has_next {
                return Ok(out);
            }
            if page == max_pages {
                // No further request follows, so the cursor is not needed: report the truncation.
                break;
            }
            // A next page without a new cursor would refetch the same page (or end silently).
            match cursor {
                Some(cursor) if after.as_str() != Some(cursor) => after = json!(cursor),
                _ => {
                    return Err(SymphonyError::TrackerError {
                        code: "linear_pagination_error".to_string(),
                        message: "Linear reported another page without a new endCursor".to_string(),
                    })
                }
            }
        }
        warn!(
            project = %self.project_slug,
            pages = max_pages,
            "Linear issue listing truncated; only the first pages are considered"
        );
        Ok(out)
    }
}

#[async_trait]
impl TrackerAdapter for LinearTracker {
    fn kind(&self) -> &'static str {
        "linear"
    }

    async fn fetch_candidate_issues(
        &self,
        active_states: &[String],
        required_labels: &[String],
    ) -> SymphonyResult<Vec<Issue>> {
        let issues = self.list_issues(active_states, MAX_CANDIDATE_PAGES).await?;
        Ok(issues
            .into_iter()
            .filter(|i| i.dispatchable && i.has_required_labels(required_labels))
            .collect())
    }

    async fn fetch_issues_by_states(&self, state_names: &[String]) -> SymphonyResult<Vec<Issue>> {
        self.list_issues(state_names, MAX_ISSUE_PAGES).await
    }

    async fn fetch_issue_states_by_ids(&self, issue_ids: &[String]) -> SymphonyResult<Vec<Issue>> {
        if issue_ids.is_empty() {
            return Ok(Vec::new());
        }
        let body = self
            .graphql(
                LINEAR_BY_IDS_QUERY,
                json!({
                    "ids": issue_ids,
                    "first": issue_ids.len().min(50),
                    "relationFirst": 50
                }),
            )
            .await?;

        let nodes = body
            .get("data")
            .and_then(|d| d.get("issues"))
            .and_then(|i| i.get("nodes"))
            .and_then(|n| n.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(nodes
            .iter()
            .filter_map(|n| self.normalize_issue(n))
            .collect())
    }

    async fn create_comment(&self, issue_id: &str, body: &str) -> SymphonyResult<()> {
        let mutation = r#"
        mutation SymphonyComment($issueId: String!, $body: String!) {
          commentCreate(input: {issueId: $issueId, body: $body}) {
            success
          }
        }
        "#;
        let _ = self
            .graphql(mutation, json!({ "issueId": issue_id, "body": body }))
            .await?;
        Ok(())
    }

    async fn update_issue_state(&self, _issue_id: &str, _state_name: &str) -> SymphonyResult<()> {
        Ok(())
    }
}

fn parse_iso_datetime(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn derive_priority_from_labels(labels: &[String]) -> Option<i32> {
    for l in labels {
        match l.as_str() {
            "p0" | "p1" | "priority:urgent" | "priority:high" | "urgent" | "critical" => {
                return Some(1)
            }
            "p2" | "priority:medium" => return Some(2),
            "p3" | "priority:low" => return Some(3),
            "p4" | "priority:trivial" => return Some(4),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_runs_are_summarised_without_inventing_a_pass() {
        let run = |status: &str, conclusion: Option<&str>| json!({ "status": status, "conclusion": conclusion });
        assert_eq!(
            summarize_check_runs(&[]),
            "none",
            "no CI is not a passing CI"
        );
        assert_eq!(
            summarize_check_runs(&[
                run("completed", Some("success")),
                run("completed", Some("skipped"))
            ]),
            "passed"
        );
        assert_eq!(
            summarize_check_runs(&[run("completed", Some("success")), run("in_progress", None)]),
            "pending"
        );
        assert_eq!(
            summarize_check_runs(&[
                run("completed", Some("success")),
                run("completed", Some("failure"))
            ]),
            "failed"
        );
        for bad in [
            "cancelled",
            "timed_out",
            "action_required",
            "startup_failure",
        ] {
            assert_eq!(
                summarize_check_runs(&[run("completed", Some(bad))]),
                "failed",
                "{bad}"
            );
        }
        assert_eq!(
            summarize_check_runs(&[run("completed", Some("something_new"))]),
            "unknown"
        );
    }

    #[test]
    fn commit_statuses_count_and_combine_with_check_runs() {
        assert_eq!(summarize_commit_statuses(&json!({})), "none");
        assert_eq!(
            summarize_commit_statuses(
                &json!({ "state": "pending", "statuses": [], "total_count": 0 })
            ),
            "none",
            "GitHub reports `pending` for a commit without any status"
        );
        let combined = |state: &str| json!({ "state": state, "statuses": [{ "context": "ci/jenkins" }], "total_count": 1 });
        assert_eq!(summarize_commit_statuses(&combined("success")), "passed");
        assert_eq!(summarize_commit_statuses(&combined("pending")), "pending");
        assert_eq!(summarize_commit_statuses(&combined("failure")), "failed");
        assert_eq!(summarize_commit_statuses(&combined("error")), "failed");
        assert_eq!(summarize_commit_statuses(&combined("weird")), "unknown");

        assert_eq!(combine_ci_signals("passed", "failed"), "failed");
        assert_eq!(combine_ci_signals("failed", "unknown"), "failed");
        assert_eq!(combine_ci_signals("passed", "unknown"), "unknown");
        assert_eq!(combine_ci_signals("none", "pending"), "pending");
        assert_eq!(combine_ci_signals("passed", "pending"), "pending");
        assert_eq!(combine_ci_signals("none", "passed"), "passed");
        assert_eq!(combine_ci_signals("passed", "none"), "passed");
        assert_eq!(combine_ci_signals("none", "none"), "none");
    }

    #[test]
    fn test_github_issue_normalization_and_pr_exclusion() {
        let tracker = GitHubTracker::new(
            None,
            "openai/symphony".to_string(),
            Some("test_tok".to_string()),
            None,
        );

        let issue_json = json!({
            "number": 42,
            "title": "Add Rust worktree manager",
            "body": "Implement isolated worktrees",
            "state": "open",
            "html_url": "https://github.com/openai/symphony/issues/42",
            "labels": [{"name": "Symphony"}, {"name": "P1"}],
            "created_at": "2026-02-01T10:00:00Z",
            "updated_at": "2026-02-01T11:00:00Z"
        });

        let normalized = tracker.normalize_issue(&issue_json).unwrap();
        assert_eq!(normalized.id, "42");
        assert_eq!(normalized.identifier, "GH-42");
        assert_eq!(normalized.priority, Some(1));
        assert!(normalized.dispatchable);
        assert_eq!(normalized.labels, vec!["symphony", "p1"]);

        // Pull request items returned by `/issues` must have dispatchable = false
        let pr_json = json!({
            "number": 43,
            "title": "PR item",
            "state": "open",
            "pull_request": {"url": "https://api.github.com/repos/openai/symphony/pulls/43"}
        });
        let norm_pr = tracker.normalize_issue(&pr_json).unwrap();
        assert!(!norm_pr.dispatchable);
    }

    #[test]
    fn test_linear_issue_normalization_and_blocker_check() {
        let tracker = LinearTracker::new(
            None,
            "symphony".to_string(),
            Some("lin_key".to_string()),
            vec!["Done".to_string(), "Cancelled".to_string()],
        );

        let raw = json!({
            "id": "lin-uuid-1",
            "identifier": "SYM-10",
            "title": "Implement scheduler",
            "description": "Spec section 8",
            "priority": 2,
            "state": {"name": "Todo"},
            "branchName": "sym/sym-10",
            "url": "https://linear.app/symphony/issue/SYM-10",
            "labels": {"nodes": [{"name": "Backend"}]},
            "inverseRelations": {
                "nodes": [
                    {
                        "type": "blocks",
                        "issue": {
                            "id": "lin-uuid-0",
                            "identifier": "SYM-09",
                            "state": {"name": "In Progress"}
                        }
                    }
                ]
            },
            "createdAt": "2026-01-15T12:00:00Z",
            "updatedAt": "2026-01-15T12:30:00Z"
        });

        let issue = tracker.normalize_issue(&raw).unwrap();
        assert_eq!(issue.identifier, "SYM-10");
        assert_eq!(issue.blocked_by.len(), 1);
        assert_eq!(issue.blocked_by[0].identifier.as_deref(), Some("SYM-09"));
        // Todo issue blocked by non-terminal SYM-09 is not dispatchable
        assert!(!issue.dispatchable);
    }
}
