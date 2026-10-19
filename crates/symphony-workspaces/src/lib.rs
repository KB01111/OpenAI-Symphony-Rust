use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use symphony_core::{
    ChangedFileSummary, HooksConfig, SymphonyError, SymphonyResult, ValidationResult,
};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, info, warn};

const HASH_PREFIX_HEX_LEN: usize = 16; // 64 bits of entropy per SPEC.md Section 4.2

/// Secret environment variables that must NEVER be inherited by workspace hooks or agent subprocesses.
pub const SECRET_ENV_VARS_TO_STRIP: &[&str] = &[
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "LINEAR_API_KEY",
    "SYMPHONY_REMOTE_TOKEN",
];

/// Compute deterministic, collision-resistant workspace directory key from `issue.identifier` (SPEC.md Section 4.2).
/// - Replaces any character outside `[A-Za-z0-9._-]` with `_`.
/// - If sanitization altered the identifier, appends `--<16-char SHA-256 hex prefix>`.
pub fn workspace_key_for_identifier(identifier: &str) -> String {
    let raw = if identifier.trim().is_empty() {
        "issue"
    } else {
        identifier.trim()
    };

    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let safe_base = if sanitized.is_empty() {
        "issue".to_string()
    } else {
        sanitized
    };

    if safe_base == raw {
        safe_base
    } else {
        let mut hasher = Sha256::new();
        hasher.update(raw.as_bytes());
        let digest = hex::encode(hasher.finalize());
        let prefix = &digest[..HASH_PREFIX_HEX_LEN];
        format!("{safe_base}--{prefix}")
    }
}

/// Deterministic branch name for an issue identifier (`symphony/<sanitized-identifier>`)
pub fn branch_name_for_identifier(identifier: &str, custom_branch: Option<&str>) -> String {
    if let Some(b) = custom_branch {
        let trimmed = b.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let clean: String = identifier
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let collapsed = clean.trim_matches('-');
    if collapsed.is_empty() {
        "symphony/task".to_string()
    } else {
        format!("symphony/{collapsed}")
    }
}

/// Normalize a path lexically without requiring the final leaf to exist yet,
/// resolving `.` and `..` components safely.
pub fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => out.push(c),
        }
    }
    out
}

/// Strip Windows extended-length path prefix (`\\?\`) for consistent comparisons.
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path
    }
}

/// Validate that `workspace_path` is strictly inside `workspace_root`, does not equal `workspace_root`,
/// and does not escape `workspace_root` via symlinks (SPEC.md Section 9.5).
pub fn validate_workspace_path(
    workspace_path: &Path,
    workspace_root: &Path,
) -> SymphonyResult<PathBuf> {
    let abs_root = if workspace_root.is_absolute() {
        lexical_normalize(workspace_root)
    } else {
        lexical_normalize(&std::env::current_dir()?.join(workspace_root))
    };

    let abs_ws = if workspace_path.is_absolute() {
        lexical_normalize(workspace_path)
    } else {
        lexical_normalize(&std::env::current_dir()?.join(workspace_path))
    };

    let canon_root = if abs_root.exists() {
        strip_verbatim_prefix(abs_root.canonicalize().unwrap_or_else(|_| abs_root.clone()))
    } else {
        abs_root.clone()
    };

    let canon_ws = if abs_ws.exists() {
        strip_verbatim_prefix(abs_ws.canonicalize().unwrap_or_else(|_| abs_ws.clone()))
    } else {
        // Resolve canonical parent if it exists
        if let Some(parent) = abs_ws.parent() {
            if parent.exists() {
                if let Ok(cp) = parent.canonicalize() {
                    if let Some(name) = abs_ws.file_name() {
                        strip_verbatim_prefix(cp).join(name)
                    } else {
                        abs_ws.clone()
                    }
                } else {
                    abs_ws.clone()
                }
            } else {
                abs_ws.clone()
            }
        } else {
            abs_ws.clone()
        }
    };

    if canon_ws == canon_root {
        return Err(SymphonyError::WorkspaceEqualsRoot(
            canon_ws.display().to_string(),
        ));
    }

    if !canon_ws.starts_with(&canon_root) {
        return Err(SymphonyError::WorkspaceOutsideRoot {
            workspace: canon_ws.display().to_string(),
            root: canon_root.display().to_string(),
        });
    }

    // Check if the target itself is a symlink pointing outside
    if let Ok(meta) = std::fs::symlink_metadata(&abs_ws) {
        if meta.file_type().is_symlink() {
            let resolved =
                std::fs::canonicalize(&abs_ws).map_err(|_| SymphonyError::SymlinkEscape {
                    workspace: abs_ws.display().to_string(),
                    root: canon_root.display().to_string(),
                })?;
            let clean_resolved = strip_verbatim_prefix(resolved);
            if !clean_resolved.starts_with(&canon_root) {
                return Err(SymphonyError::SymlinkEscape {
                    workspace: clean_resolved.display().to_string(),
                    root: canon_root.display().to_string(),
                });
            }
        }
    }

    Ok(canon_ws)
}

#[derive(Debug, Clone)]
pub struct AllocatedWorkspace {
    pub key: String,
    pub path: PathBuf,
    pub branch_name: String,
    pub created_newly: bool,
    pub is_git_worktree: bool,
}

#[derive(Clone, Default)]
pub struct WorkspaceManager;

impl WorkspaceManager {
    pub fn new() -> Self {
        Self
    }

    /// Create or reuse an isolated workspace for an issue/task.
    /// If `repo_path` points to a valid Git repository, creates or reuses an isolated Git worktree
    /// on a dedicated task branch. Otherwise creates an isolated directory under `workspace_root`.
    pub async fn prepare_workspace(
        &self,
        identifier: &str,
        workspace_root: &Path,
        repo_path: Option<&Path>,
        base_branch: &str,
        custom_branch: Option<&str>,
        hooks: &HooksConfig,
    ) -> SymphonyResult<AllocatedWorkspace> {
        std::fs::create_dir_all(workspace_root)?;

        let key = workspace_key_for_identifier(identifier);
        let candidate_path = workspace_root.join(&key);
        let validated_path = validate_workspace_path(&candidate_path, workspace_root)?;
        let branch_name = branch_name_for_identifier(identifier, custom_branch);

        let existed = validated_path.exists();
        let mut is_git_worktree = false;

        if let Some(repo) = repo_path {
            if repo.exists() && repo.join(".git").exists() {
                is_git_worktree = true;
                if !existed {
                    self.create_git_worktree(repo, &validated_path, &branch_name, base_branch)
                        .await?;
                } else {
                    debug!(
                        workspace = %validated_path.display(),
                        branch = %branch_name,
                        "Reusing existing Git worktree workspace"
                    );
                }
            }
        }

        if !validated_path.exists() {
            std::fs::create_dir_all(&validated_path)?;
        }

        let created_newly = !existed;

        // Clean temporary artifacts as reference implementation does
        for tmp_dir in [".elixir_ls", "tmp"] {
            let p = validated_path.join(tmp_dir);
            if p.exists() {
                let _ = std::fs::remove_dir_all(p);
            }
        }

        if created_newly {
            if let Some(script) = &hooks.after_create {
                if let Err(err) =
                    run_lifecycle_hook("after_create", script, &validated_path, hooks.timeout_ms)
                        .await
                {
                    // Failure on after_create is fatal and removes partially initialized directory
                    let _ = self
                        .cleanup_workspace(&validated_path, workspace_root, repo_path, hooks, true)
                        .await;
                    return Err(err);
                }
            }
        }

        Ok(AllocatedWorkspace {
            key,
            path: validated_path,
            branch_name,
            created_newly,
            is_git_worktree,
        })
    }

    async fn create_git_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> SymphonyResult<()> {
        // First prune stale worktree metadata
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("worktree")
            .arg("prune")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        // Check if base_branch resolves; otherwise fallback to HEAD
        let base_ref = if Self::git_ref_exists(repo_path, base_branch).await {
            base_branch.to_string()
        } else {
            "HEAD".to_string()
        };

        let output = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("worktree")
            .arg("add")
            .arg("-B")
            .arg(branch_name)
            .arg(worktree_path)
            .arg(&base_ref)
            .output()
            .await
            .map_err(|e| SymphonyError::GitWorktreeError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // If branch is already checked out elsewhere, fallback to detached or unique branch suffix
            if stderr.contains("already checked out") || stderr.contains("already used by worktree")
            {
                let fallback_branch = format!("{branch_name}-{}", std::process::id());
                let retry_out = Command::new("git")
                    .arg("-C")
                    .arg(repo_path)
                    .arg("worktree")
                    .arg("add")
                    .arg("-B")
                    .arg(&fallback_branch)
                    .arg(worktree_path)
                    .arg(&base_ref)
                    .output()
                    .await
                    .map_err(|e| SymphonyError::GitWorktreeError(e.to_string()))?;

                if !retry_out.status.success() {
                    return Err(SymphonyError::GitWorktreeError(
                        String::from_utf8_lossy(&retry_out.stderr)
                            .trim()
                            .to_string(),
                    ));
                }
            } else {
                return Err(SymphonyError::GitWorktreeError(stderr.trim().to_string()));
            }
        }

        info!(
            repo = %repo_path.display(),
            worktree = %worktree_path.display(),
            branch = %branch_name,
            "Created isolated Git worktree"
        );
        Ok(())
    }

    async fn git_ref_exists(repo_path: &Path, git_ref: &str) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("rev-parse")
            .arg("--verify")
            .arg(git_ref)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if a Git worktree has uncommitted modifications.
    pub async fn is_worktree_dirty(&self, workspace_path: &Path) -> bool {
        if !workspace_path.exists() {
            return false;
        }
        let output = Command::new("git")
            .arg("-C")
            .arg(workspace_path)
            .arg("status")
            .arg("--porcelain")
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => {
                !String::from_utf8_lossy(&out.stdout).trim().is_empty()
            }
            _ => false,
        }
    }

    /// Inspect modified/added/deleted files in the workspace worktree.
    pub async fn inspect_changed_files(
        &self,
        workspace_path: &Path,
    ) -> SymphonyResult<Vec<ChangedFileSummary>> {
        if !workspace_path.exists() {
            return Ok(Vec::new());
        }
        let status_out = Command::new("git")
            .arg("-C")
            .arg(workspace_path)
            .arg("status")
            .arg("--porcelain")
            .output()
            .await;

        let Ok(out) = status_out else {
            return Ok(Vec::new());
        };
        if !out.status.success() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if line.len() < 4 {
                continue;
            }
            let code = line[..2].trim().to_string();
            let path = line[3..].trim().to_string();
            let status = match code.as_str() {
                "M" | "MM" => "modified",
                "A" | "??" => "added",
                "D" => "deleted",
                "R" => "renamed",
                _ => "changed",
            }
            .to_string();

            files.push(ChangedFileSummary {
                path,
                status,
                additions: 1,
                deletions: 0,
            });
        }
        Ok(files)
    }

    /// Get unified diff preview of the workspace.
    pub async fn get_diff_preview(&self, workspace_path: &Path) -> SymphonyResult<String> {
        if !workspace_path.exists() {
            return Ok(String::new());
        }
        let out = Command::new("git")
            .arg("-C")
            .arg(workspace_path)
            .arg("diff")
            .arg("HEAD")
            .output()
            .await;

        match out {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout).to_string();
                Ok(s.chars().take(32_000).collect())
            }
            _ => Ok(String::new()),
        }
    }

    /// Safely remove a workspace directory and detach its Git worktree registration.
    /// If `force` is false and the worktree has uncommitted user work, refuses deletion.
    pub async fn cleanup_workspace(
        &self,
        workspace_path: &Path,
        workspace_root: &Path,
        repo_path: Option<&Path>,
        hooks: &HooksConfig,
        force: bool,
    ) -> SymphonyResult<()> {
        if !workspace_path.exists() {
            return Ok(());
        }

        let validated = validate_workspace_path(workspace_path, workspace_root)?;

        if !force && self.is_worktree_dirty(&validated).await {
            return Err(SymphonyError::GitWorktreeError(format!(
                "Refusing to delete dirty worktree without authorization: {}",
                validated.display()
            )));
        }

        if let Some(script) = &hooks.before_remove {
            if let Err(e) =
                run_lifecycle_hook("before_remove", script, &validated, hooks.timeout_ms).await
            {
                warn!(error = %e, "before_remove hook failed (logged and ignored per SPEC.md)");
            }
        }

        if let Some(repo) = repo_path {
            if repo.exists() {
                let mut cmd = Command::new("git");
                cmd.arg("-C").arg(repo).arg("worktree").arg("remove");
                if force {
                    cmd.arg("--force");
                }
                cmd.arg(&validated);
                let _ = cmd.output().await;
            }
        }

        if validated.exists() {
            std::fs::remove_dir_all(&validated)?;
        }

        Ok(())
    }

    /// Execute validation commands inside the isolated workspace and return real execution results (Section 10).
    pub async fn run_validation_commands(
        &self,
        task_id: &str,
        attempt_number: u32,
        workspace_path: &Path,
        commands: &[String],
        timeout_ms: u64,
    ) -> Vec<ValidationResult> {
        let mut results = Vec::new();
        for cmd_str in commands {
            let trimmed = cmd_str.trim();
            if trimmed.is_empty() {
                continue;
            }
            let start = std::time::Instant::now();
            let executed_at = chrono::Utc::now();

            let mut command = build_host_shell_command(trimmed, workspace_path);
            let res = timeout(Duration::from_millis(timeout_ms), command.output()).await;

            let duration_ms = start.elapsed().as_millis() as u64;
            match res {
                Ok(Ok(output)) => {
                    let code = output.status.code().unwrap_or(-1);
                    let passed = output.status.success();
                    results.push(ValidationResult {
                        id: format!("{task_id}-val-{}", results.len() + 1),
                        task_id: task_id.to_string(),
                        attempt_number,
                        command: trimmed.to_string(),
                        exit_code: code,
                        passed,
                        stdout: String::from_utf8_lossy(&output.stdout)
                            .chars()
                            .take(8192)
                            .collect(),
                        stderr: String::from_utf8_lossy(&output.stderr)
                            .chars()
                            .take(8192)
                            .collect(),
                        duration_ms,
                        executed_at,
                    });
                }
                Ok(Err(e)) => {
                    results.push(ValidationResult {
                        id: format!("{task_id}-val-{}", results.len() + 1),
                        task_id: task_id.to_string(),
                        attempt_number,
                        command: trimmed.to_string(),
                        exit_code: -1,
                        passed: false,
                        stdout: String::new(),
                        stderr: format!("Process launch error: {e}"),
                        duration_ms,
                        executed_at,
                    });
                }
                Err(_) => {
                    results.push(ValidationResult {
                        id: format!("{task_id}-val-{}", results.len() + 1),
                        task_id: task_id.to_string(),
                        attempt_number,
                        command: trimmed.to_string(),
                        exit_code: -1,
                        passed: false,
                        stdout: String::new(),
                        stderr: format!("Command timed out after {timeout_ms} ms"),
                        duration_ms,
                        executed_at,
                    });
                }
            }
        }
        results
    }
}

/// Run a repository lifecycle hook (`after_create`, `before_run`, `after_run`, `before_remove`)
/// inside `workspace_path` with sanitized environment variables and timeout enforcement.
pub async fn run_lifecycle_hook(
    hook_name: &str,
    script: &str,
    workspace_path: &Path,
    timeout_ms: u64,
) -> SymphonyResult<String> {
    info!(
        hook = hook_name,
        workspace = %workspace_path.display(),
        "Running workspace lifecycle hook"
    );

    let mut cmd = build_host_shell_command(script, workspace_path);
    let fut = cmd.output();

    match timeout(Duration::from_millis(timeout_ms), fut).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                Err(SymphonyError::HookFailed {
                    hook: hook_name.to_string(),
                    reason: format!(
                        "exit status {:?}: {} {}",
                        output.status.code(),
                        stdout.trim(),
                        stderr.trim()
                    ),
                })
            }
        }
        Ok(Err(e)) => Err(SymphonyError::HookFailed {
            hook: hook_name.to_string(),
            reason: e.to_string(),
        }),
        Err(_) => Err(SymphonyError::HookTimeout {
            hook: hook_name.to_string(),
            timeout_ms,
        }),
    }
}

/// Build a host-compatible command on Windows (PowerShell) or Unix (`sh -lc`)
/// with secret tracker environment variables explicitly removed.
pub fn build_host_shell_command(script: &str, cwd: &Path) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("powershell.exe");
        c.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-lc").arg(script);
        c
    };

    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    for var in SECRET_ENV_VARS_TO_STRIP {
        cmd.env_remove(var);
    }

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_workspace_key_sanitization_and_collision_resistance() {
        // Already clean identifier stays unchanged
        assert_eq!(workspace_key_for_identifier("ABC-123"), "ABC-123");
        assert_eq!(workspace_key_for_identifier("repo.1_2-3"), "repo.1_2-3");

        // Identifiers with disallowed chars get sanitized AND suffixed with 16-char SHA-256 hex
        let k1 = workspace_key_for_identifier("owner/repo#42");
        let k2 = workspace_key_for_identifier("owner_repo_42");
        let k3 = workspace_key_for_identifier("owner:repo?42");

        assert!(k1.starts_with("owner_repo_42--"));
        assert_eq!(k1.len(), "owner_repo_42--".len() + 16);
        assert_eq!(k2, "owner_repo_42");
        assert_ne!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[test]
    fn test_workspace_path_containment() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("workspaces");
        std::fs::create_dir_all(&root).unwrap();

        let valid = root.join("GH-100");
        assert!(validate_workspace_path(&valid, &root).is_ok());

        // Equal to root is rejected
        let eq_err = validate_workspace_path(&root, &root).unwrap_err();
        assert_eq!(eq_err.code(), "workspace_equals_root");

        // Outside root via .. is rejected
        let escape = root.join("..").join("outside");
        let esc_err = validate_workspace_path(&escape, &root).unwrap_err();
        assert_eq!(esc_err.code(), "workspace_outside_root");
    }

    #[tokio::test]
    async fn test_git_worktree_lifecycle() {
        let temp = tempdir().unwrap();
        let repo_dir = temp.path().join("repo");
        let ws_root = temp.path().join("workspaces");
        std::fs::create_dir_all(&repo_dir).unwrap();

        // Initialize a real temporary git repository
        Command::new("git")
            .arg("init")
            .arg("-b")
            .arg("main")
            .current_dir(&repo_dir)
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@symphony.local"])
            .current_dir(&repo_dir)
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Symphony Test"])
            .current_dir(&repo_dir)
            .output()
            .await
            .unwrap();
        std::fs::write(repo_dir.join("README.md"), "# Test Repo\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_dir)
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&repo_dir)
            .output()
            .await
            .unwrap();

        let mgr = WorkspaceManager::new();
        let hooks = HooksConfig {
            after_create: None,
            before_run: None,
            after_run: None,
            before_remove: None,
            timeout_ms: 10_000,
        };

        let allocated = mgr
            .prepare_workspace("GH-101", &ws_root, Some(&repo_dir), "main", None, &hooks)
            .await
            .unwrap();

        assert!(allocated.created_newly);
        assert!(allocated.is_git_worktree);
        assert!(allocated.path.join("README.md").exists());

        // Modify a file inside the isolated worktree
        std::fs::write(allocated.path.join("feature.txt"), "hello world\n").unwrap();
        assert!(mgr.is_worktree_dirty(&allocated.path).await);

        let changed = mgr.inspect_changed_files(&allocated.path).await.unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].path, "feature.txt");

        // Refuses non-forced cleanup on dirty worktree
        assert!(mgr
            .cleanup_workspace(&allocated.path, &ws_root, Some(&repo_dir), &hooks, false)
            .await
            .is_err());

        // Forced cleanup succeeds
        mgr.cleanup_workspace(&allocated.path, &ws_root, Some(&repo_dir), &hooks, true)
            .await
            .unwrap();
        assert!(!allocated.path.exists());
    }
}
