use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use symphony_core::{
    ChangedFileSummary, HooksConfig, SymphonyError, SymphonyResult, ValidationResult,
};
use tokio::process::Command;
use tracing::{debug, info, warn};

pub mod process;
pub use process::{hide_console_window, output_with_timeout, prepare_tree_command, ProcessTree};

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

const MAX_UNTRACKED_DIFF_FILES: usize = 200;

/// An empty directory used as `core.hooksPath`, so no repository hook can run during a git command
/// that Symphony issues on the user's behalf. Removed on drop.
struct SafeHooksDir(PathBuf);

impl SafeHooksDir {
    fn new() -> SymphonyResult<Self> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("symphony-no-hooks-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    /// `-c` options that turn hooks and the fsmonitor hook off.
    fn config_args(&self) -> Vec<String> {
        vec![
            "-c".into(),
            format!("core.hooksPath={}", self.0.display()),
            "-c".into(),
            "core.fsmonitor=false".into(),
        ]
    }
}

impl Drop for SafeHooksDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `https://host/` of an `https://` remote URL, the scope of git's per-URL `http.<url>.extraheader`.
fn https_origin(remote: &str) -> Option<String> {
    let url = url::Url::parse(remote).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    Some(format!("https://{}/", url.host_str()?))
}

/// Result of simulating a merge of a worktree into its base branch ([`WorkspaceManager::check_mergeability`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeCheck {
    /// The changes merge cleanly into the base branch as it is now.
    Clean,
    /// Merging conflicts in these paths.
    Conflicts(Vec<String>),
    /// The check could not run; the reason is human-readable. Mergeability is unknown.
    Unavailable(String),
}

/// `git` without a console window (the release desktop app is a GUI-subsystem process).
pub fn git_command() -> Command {
    let mut cmd = Command::new("git");
    hide_console_window(&mut cmd);
    cmd
}

async fn git_stdout(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = git_command()
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Run git and read at most `limit` bytes of stdout without buffering the rest; the child is
/// killed once the budget is exhausted. Returns the bytes read and whether output was cut off.
/// Non-zero exit codes are accepted (`git diff --no-index` exits 1 when files differ).
async fn git_stdout_bounded(cwd: &Path, args: &[&str], limit: usize) -> (Vec<u8>, bool) {
    use tokio::io::AsyncReadExt;
    let child = git_command()
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let Ok(mut child) = child else {
        return (Vec::new(), false);
    };
    let Some(stdout) = child.stdout.take() else {
        return (Vec::new(), false);
    };
    let mut buf = Vec::new();
    let read = stdout.take(limit as u64 + 1).read_to_end(&mut buf).await;
    let overflow = read.is_ok() && buf.len() > limit;
    if overflow {
        buf.truncate(limit);
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
    (buf, overflow)
}

/// Split NUL-terminated git output (`-z`) into raw, unquoted paths/records.
fn split_nul(output: &str) -> impl Iterator<Item = &str> {
    output.split('\0').filter(|s| !s.is_empty())
}

fn count_text_lines(path: &Path) -> u32 {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() <= 2 * 1024 * 1024 => std::fs::read(path)
            .map(|bytes| bytes.iter().filter(|b| **b == b'\n').count() as u32)
            .unwrap_or(0),
        _ => 0,
    }
}

fn truncate_utf8(mut text: String, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut cut = max_bytes;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    (text, true)
}

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
        let _ = git_command()
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

        let output = git_command()
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
                let retry_out = git_command()
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
        git_command()
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
        let output = git_command()
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

    /// Inspect modified/added/deleted files in the workspace worktree, including untracked files,
    /// with real line counts from `git diff --numstat HEAD`.
    pub async fn inspect_changed_files(
        &self,
        workspace_path: &Path,
    ) -> SymphonyResult<Vec<ChangedFileSummary>> {
        if !workspace_path.exists() {
            return Ok(Vec::new());
        }
        // `-z` output is NUL-separated and never C-quoted, so paths with spaces, quotes or
        // non-ASCII characters are taken verbatim.
        let Some(status) = git_stdout(
            workspace_path,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )
        .await
        else {
            return Ok(Vec::new());
        };
        let numstat = git_stdout(workspace_path, &["diff", "--numstat", "-z", "HEAD"])
            .await
            .unwrap_or_default();
        let mut counts = std::collections::HashMap::new();
        let mut records = split_nul(&numstat);
        while let Some(record) = records.next() {
            let mut parts = record.splitn(3, '\t');
            let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            // Renames: `add\tdel\t` followed by the old and new path as separate records.
            let path = if path.is_empty() {
                let _old = records.next();
                records.next().unwrap_or_default()
            } else {
                path
            };
            counts.insert(
                path.to_string(),
                (
                    add.parse::<u32>().unwrap_or(0),
                    del.parse::<u32>().unwrap_or(0),
                ),
            );
        }

        let mut files = Vec::new();
        let mut entries = split_nul(&status);
        while let Some(entry) = entries.next() {
            if entry.len() < 4 {
                continue;
            }
            let code = entry[..2].trim().to_string();
            // Renames/copies list the new path first, then the original path as its own record.
            if code.starts_with('R') || code.starts_with('C') {
                let _original = entries.next();
            }
            let path = entry[3..].to_string();
            let status = match code.as_str() {
                "M" | "MM" | "AM" => "modified",
                "A" | "??" => "added",
                "D" => "deleted",
                c if c.starts_with('R') => "renamed",
                _ => "changed",
            }
            .to_string();
            let (additions, deletions) = match counts.get(&path) {
                Some(&(a, d)) => (a, d),
                None if code == "??" => (count_text_lines(&workspace_path.join(&path)), 0),
                None => (0, 0),
            };
            files.push(ChangedFileSummary {
                path,
                status,
                additions,
                deletions,
            });
        }
        Ok(files)
    }

    /// Get unified diff preview of the workspace (tracked changes plus untracked files), capped at 32 KB.
    pub async fn get_diff_preview(&self, workspace_path: &Path) -> SymphonyResult<String> {
        Ok(self.collect_diff(workspace_path, 32_000).await.0)
    }

    /// Unified diff of tracked changes against `HEAD` followed by diffs for untracked files.
    /// Returns the diff (truncated on a UTF-8 boundary to `max_bytes`) and whether truncation occurred.
    pub async fn collect_diff(&self, workspace_path: &Path, max_bytes: usize) -> (String, bool) {
        if !workspace_path.exists() {
            return (String::new(), false);
        }
        let (tracked, mut truncated) =
            git_stdout_bounded(workspace_path, &["diff", "HEAD"], max_bytes).await;
        let mut diff = String::from_utf8_lossy(&tracked).into_owned();
        let untracked = git_stdout(
            workspace_path,
            &["ls-files", "-z", "--others", "--exclude-standard"],
        )
        .await
        .unwrap_or_default();
        let paths: Vec<&str> = split_nul(&untracked).collect();
        for (index, path) in paths.iter().enumerate() {
            if truncated || diff.len() >= max_bytes {
                truncated = true;
                break;
            }
            if index >= MAX_UNTRACKED_DIFF_FILES {
                // Remaining untracked files are omitted, so the diff is incomplete.
                truncated = true;
                break;
            }
            let budget = max_bytes - diff.len();
            let (bytes, cut) = git_stdout_bounded(
                workspace_path,
                &["diff", "--no-index", "--", "/dev/null", path],
                budget,
            )
            .await;
            diff.push_str(&String::from_utf8_lossy(&bytes));
            truncated |= cut;
        }
        let (diff, cut) = truncate_utf8(diff, max_bytes);
        (diff, truncated || cut)
    }

    /// Simulate merging the worktree's current state (committed and uncommitted changes, untracked
    /// files included, `.gitignore` honoured) into `base_ref`, without touching the worktree, its
    /// index or any branch. Uses `git merge-tree --write-tree` (Git 2.38+); the helper objects it
    /// writes are unreferenced and left to `git gc`.
    pub async fn check_mergeability(&self, workspace_path: &Path, base_ref: &str) -> MergeCheck {
        match Self::simulate_merge(workspace_path, base_ref).await {
            Ok(check) => check,
            Err(reason) => MergeCheck::Unavailable(reason),
        }
    }

    async fn simulate_merge(workspace_path: &Path, base_ref: &str) -> Result<MergeCheck, String> {
        if !workspace_path.exists() {
            return Err("the workspace no longer exists".into());
        }
        let base = if Self::git_ref_exists(workspace_path, base_ref).await {
            base_ref.to_string()
        } else if Self::git_ref_exists(workspace_path, &format!("origin/{base_ref}")).await {
            format!("origin/{base_ref}")
        } else {
            return Err(format!("base branch `{base_ref}` was not found"));
        };

        let git_dir = git_stdout(workspace_path, &["rev-parse", "--absolute-git-dir"])
            .await
            .ok_or("not a Git worktree")?;
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let index = PathBuf::from(git_dir.trim()).join(format!(
            "symphony-merge-check-{}-{unique}.index",
            std::process::id()
        ));
        let result = Self::merge_tree_of_snapshot(workspace_path, &base, &index).await;
        let _ = std::fs::remove_file(&index);
        result
    }

    async fn merge_tree_of_snapshot(
        workspace_path: &Path,
        base: &str,
        index: &Path,
    ) -> Result<MergeCheck, String> {
        // A private index holds the worktree's state, so the real index is never modified.
        let run = |args: Vec<&str>, with_index: bool| {
            let mut cmd = git_command();
            cmd.arg("-C")
                .arg(workspace_path)
                .args(args)
                .stdin(Stdio::null())
                // The snapshot commit only exists to be merged: any identity will do.
                .env("GIT_AUTHOR_NAME", "Symphony")
                .env("GIT_AUTHOR_EMAIL", "symphony@localhost")
                .env("GIT_COMMITTER_NAME", "Symphony")
                .env("GIT_COMMITTER_EMAIL", "symphony@localhost");
            if with_index {
                cmd.env("GIT_INDEX_FILE", index);
            }
            async move {
                cmd.output()
                    .await
                    .map_err(|e| format!("could not run git: {e}"))
            }
        };
        let stdout_of = |out: &std::process::Output, what: &str| -> Result<String, String> {
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                Err(format!(
                    "{what} failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ))
            }
        };

        stdout_of(
            &run(vec!["read-tree", "HEAD"], true).await?,
            "git read-tree",
        )?;
        stdout_of(&run(vec!["add", "-A"], true).await?, "git add")?;
        let tree = stdout_of(&run(vec!["write-tree"], true).await?, "git write-tree")?;
        let commit = stdout_of(
            &run(
                vec![
                    "commit-tree",
                    &tree,
                    "-p",
                    "HEAD",
                    "-m",
                    "symphony merge check",
                ],
                false,
            )
            .await?,
            "git commit-tree",
        )?;
        let merged = run(
            vec![
                "merge-tree",
                "--write-tree",
                "--name-only",
                "--no-messages",
                base,
                &commit,
            ],
            false,
        )
        .await?;
        match merged.status.code() {
            Some(0) => Ok(MergeCheck::Clean),
            // Exit status 1: the merge has conflicts; the output is the tree id followed by the
            // conflicted paths.
            Some(1) => {
                let stdout = String::from_utf8_lossy(&merged.stdout);
                let files = stdout
                    .lines()
                    .skip(1)
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect();
                Ok(MergeCheck::Conflicts(files))
            }
            _ => Err(format!(
                "git merge-tree is unavailable or failed (Git 2.38 or newer is required): {}",
                String::from_utf8_lossy(&merged.stderr).trim()
            )),
        }
    }

    /// Commit every change in the worktree (untracked files included, `.gitignore` honoured) to
    /// `branch`, which must be the branch the worktree has checked out. Returns the new commit, or
    /// `None` when there was nothing to commit. Repository hooks and `core.fsmonitor` never run:
    /// the worktree is the agent's, and a commit made on the user's behalf must not execute what the
    /// agent left behind.
    pub async fn commit_changes(
        &self,
        workspace_path: &Path,
        branch: &str,
        message: &str,
    ) -> SymphonyResult<Option<String>> {
        let hooks = SafeHooksDir::new()?;
        let current = Self::git_checked(
            workspace_path,
            &hooks,
            &[],
            &["rev-parse", "--abbrev-ref", "HEAD"],
        )
        .await?
        .trim()
        .to_string();
        if current != branch {
            return Err(SymphonyError::Conflict(format!(
                "the worktree has `{current}` checked out, not the task branch `{branch}`"
            )));
        }
        Self::git_checked(workspace_path, &hooks, &[], &["add", "-A"]).await?;
        let staged = git_command()
            .arg("-C")
            .arg(workspace_path)
            .args(hooks.config_args())
            .args(["diff", "--cached", "--quiet"])
            .stdin(Stdio::null())
            .status()
            .await
            .map_err(|e| SymphonyError::GitWorktreeError(format!("could not run git: {e}")))?;
        match staged.code() {
            Some(0) => return Ok(None),
            Some(1) => {}
            other => {
                return Err(SymphonyError::GitWorktreeError(format!(
                    "git diff --cached failed ({other:?})"
                )))
            }
        }
        // Commit under the user's own identity when git knows one, else under a neutral one.
        let has_identity = git_stdout(workspace_path, &["config", "user.email"])
            .await
            .map(|e| !e.trim().is_empty())
            .unwrap_or(false);
        let identity: Vec<(&str, &str)> = if has_identity {
            Vec::new()
        } else {
            vec![
                ("GIT_AUTHOR_NAME", "Symphony Desktop"),
                ("GIT_AUTHOR_EMAIL", "symphony@localhost"),
                ("GIT_COMMITTER_NAME", "Symphony Desktop"),
                ("GIT_COMMITTER_EMAIL", "symphony@localhost"),
            ]
        };
        Self::git_checked(
            workspace_path,
            &hooks,
            &identity,
            &["commit", "--no-verify", "--no-gpg-sign", "-m", message],
        )
        .await?;
        let sha = Self::git_checked(workspace_path, &hooks, &[], &["rev-parse", "HEAD"]).await?;
        Ok(Some(sha.trim().to_string()))
    }

    /// Push `branch` to `origin` (never forced). `token` authenticates an `https://` remote for
    /// this one command through git's environment-based configuration: it is not written to any
    /// config file and does not appear on the command line.
    pub async fn push_branch(
        &self,
        workspace_path: &Path,
        branch: &str,
        token: Option<&str>,
    ) -> SymphonyResult<()> {
        if branch.is_empty() || branch.starts_with('-') || branch.contains(char::is_whitespace) {
            return Err(SymphonyError::GitWorktreeError(format!(
                "`{branch}` is not a branch name that can be pushed"
            )));
        }
        let hooks = SafeHooksDir::new()?;
        let remote = Self::git_checked(
            workspace_path,
            &hooks,
            &[],
            &["remote", "get-url", "origin"],
        )
        .await
        .map_err(|_| {
            SymphonyError::Conflict("the repository has no `origin` remote to push to".into())
        })?
        .trim()
        .to_string();
        let mut env: Vec<(String, String)> = Vec::new();
        if let (Some(token), Some(base)) = (token, https_origin(&remote)) {
            use base64::Engine;
            let credentials =
                base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
            env.push(("GIT_CONFIG_COUNT".into(), "1".into()));
            env.push((
                "GIT_CONFIG_KEY_0".into(),
                format!("http.{base}.extraheader"),
            ));
            env.push((
                "GIT_CONFIG_VALUE_0".into(),
                format!("AUTHORIZATION: basic {credentials}"),
            ));
        }
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        let env_refs: Vec<(&str, &str)> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        Self::git_checked(
            workspace_path,
            &hooks,
            &env_refs,
            &["push", "--no-verify", "origin", &refspec],
        )
        .await
        .map(drop)
        .map_err(|e| match (e, token) {
            // Never echo the token, whatever git printed.
            (SymphonyError::GitWorktreeError(msg), Some(t)) => {
                SymphonyError::GitWorktreeError(msg.replace(t, "[REDACTED]"))
            }
            (other, _) => other,
        })
    }

    /// Run git in `cwd` with hooks and fsmonitor disabled; Ok(stdout) on success.
    async fn git_checked(
        cwd: &Path,
        hooks: &SafeHooksDir,
        env: &[(&str, &str)],
        args: &[&str],
    ) -> SymphonyResult<String> {
        let mut cmd = git_command();
        cmd.arg("-C")
            .arg(cwd)
            .args(hooks.config_args())
            .args(args)
            .stdin(Stdio::null())
            // Never block on a credential prompt: a missing credential is an error.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never");
        for (key, value) in env {
            cmd.env(key, value);
        }
        let out = cmd
            .output()
            .await
            .map_err(|e| SymphonyError::GitWorktreeError(format!("could not run git: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(SymphonyError::GitWorktreeError(format!(
                "git {} failed: {}",
                args.first().copied().unwrap_or(""),
                String::from_utf8_lossy(&out.stderr).trim()
            )))
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
                let mut cmd = git_command();
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
            let res = output_with_timeout(&mut command, Duration::from_millis(timeout_ms)).await;

            let duration_ms = start.elapsed().as_millis() as u64;
            match res {
                Some(Ok(output)) => {
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
                Some(Err(e)) => {
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
                None => {
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

    match output_with_timeout(&mut cmd, Duration::from_millis(timeout_ms)).await {
        Some(Ok(output)) => {
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
        Some(Err(e)) => Err(SymphonyError::HookFailed {
            hook: hook_name.to_string(),
            reason: e.to_string(),
        }),
        None => Err(SymphonyError::HookTimeout {
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
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-lc").arg(script);
        c
    };

    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    prepare_tree_command(&mut cmd);

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
        assert_eq!((changed[0].additions, changed[0].deletions), (1, 0));

        // Tracked modifications report real line counts and both kinds show up in the diff.
        std::fs::write(
            allocated.path.join("README.md"),
            "# Renamed Repo\nline two\n",
        )
        .unwrap();
        let mut changed = mgr.inspect_changed_files(&allocated.path).await.unwrap();
        changed.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(changed.len(), 2);
        assert_eq!(changed[0].path, "README.md");
        assert_eq!((changed[0].additions, changed[0].deletions), (2, 1));

        let (diff, truncated) = mgr.collect_diff(&allocated.path, 64_000).await;
        assert!(!truncated);
        assert!(diff.contains("+# Renamed Repo"));
        assert!(diff.contains("-# Test Repo"));
        assert!(diff.contains("feature.txt"));
        assert!(diff.contains("+hello world"));

        let (short, truncated) = mgr.collect_diff(&allocated.path, 40).await;
        assert!(truncated);
        assert!(short.len() <= 40);

        // Paths that git would C-quote (non-ASCII, spaces) are parsed verbatim via `-z`.
        std::fs::write(allocated.path.join("café notes.txt"), "bonjour\n").unwrap();
        let changed = mgr.inspect_changed_files(&allocated.path).await.unwrap();
        assert!(changed
            .iter()
            .any(|f| f.path == "café notes.txt" && f.additions == 1));
        let (diff, _) = mgr.collect_diff(&allocated.path, 64_000).await;
        assert!(diff.contains("+bonjour"));

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

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@localhost")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@localhost")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A repository on `main` holding `file.txt` and a worktree `wt` branched from it.
    fn repo_with_worktree(root: &Path) -> (PathBuf, PathBuf) {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("file.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(repo.join("other.txt"), "other\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "initial"]);
        let wt = root.join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "task",
                wt.to_str().unwrap(),
                "main",
            ],
        );
        (repo, wt)
    }

    #[tokio::test]
    async fn mergeability_is_clean_when_the_base_moved_elsewhere() {
        let dir = tempdir().unwrap();
        let (repo, wt) = repo_with_worktree(dir.path());
        // The agent edits `file.txt` (uncommitted) and adds an untracked file; main changes another file.
        std::fs::write(wt.join("file.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(wt.join("new.txt"), "new\n").unwrap();
        std::fs::write(repo.join("other.txt"), "changed upstream\n").unwrap();
        git(&repo, &["commit", "-am", "upstream change"]);

        let check = WorkspaceManager::new()
            .check_mergeability(&wt, "main")
            .await;
        assert_eq!(check, MergeCheck::Clean);
        // The check left the worktree exactly as the agent left it.
        let status = git_stdout(&wt, &["status", "--porcelain"]).await.unwrap();
        assert!(
            status.contains(" M file.txt") && status.contains("?? new.txt"),
            "{status}"
        );
        let leftovers: Vec<_> = std::fs::read_dir(
            git_stdout(&wt, &["rev-parse", "--absolute-git-dir"])
                .await
                .unwrap()
                .trim(),
        )
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("symphony-merge-check")
        })
        .collect();
        assert!(leftovers.is_empty(), "temporary index must be removed");
    }

    #[tokio::test]
    async fn mergeability_reports_the_conflicting_paths() {
        let dir = tempdir().unwrap();
        let (repo, wt) = repo_with_worktree(dir.path());
        std::fs::write(wt.join("file.txt"), "one\nAGENT\nthree\n").unwrap();
        std::fs::write(repo.join("file.txt"), "one\nUPSTREAM\nthree\n").unwrap();
        git(&repo, &["commit", "-am", "conflicting upstream change"]);

        let check = WorkspaceManager::new()
            .check_mergeability(&wt, "main")
            .await;
        assert_eq!(check, MergeCheck::Conflicts(vec!["file.txt".to_string()]));
    }

    #[tokio::test]
    async fn mergeability_is_unavailable_for_an_unknown_base_branch() {
        let dir = tempdir().unwrap();
        let (_repo, wt) = repo_with_worktree(dir.path());
        match WorkspaceManager::new()
            .check_mergeability(&wt, "no-such-branch")
            .await
        {
            MergeCheck::Unavailable(reason) => {
                assert!(reason.contains("no-such-branch"), "{reason}")
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        let gone = dir.path().join("gone");
        assert!(matches!(
            WorkspaceManager::new()
                .check_mergeability(&gone, "main")
                .await,
            MergeCheck::Unavailable(_)
        ));
    }

    /// `repo_with_worktree` plus a bare `origin` the repository pushes to.
    fn repo_with_origin(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let (repo, wt) = repo_with_worktree(root);
        let origin = root.join("origin.git");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "--bare", "-b", "main"]);
        git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        (repo, wt, origin)
    }

    #[tokio::test]
    async fn commit_changes_commits_everything_once() {
        let dir = tempdir().unwrap();
        let (_repo, wt) = repo_with_worktree(dir.path());
        let manager = WorkspaceManager::new();
        assert_eq!(
            manager
                .commit_changes(&wt, "task", "nothing")
                .await
                .unwrap(),
            None
        );

        std::fs::write(wt.join("file.txt"), "one\nchanged\nthree\n").unwrap();
        std::fs::write(wt.join("new.txt"), "new\n").unwrap();
        let sha = manager
            .commit_changes(&wt, "task", "SYM-1: Implement")
            .await
            .unwrap()
            .expect("a commit");
        let head = git_stdout(&wt, &["rev-parse", "HEAD"]).await.unwrap();
        assert_eq!(head.trim(), sha);
        let files = git_stdout(&wt, &["show", "--name-only", "--format=%s", "HEAD"])
            .await
            .unwrap();
        assert!(files.starts_with("SYM-1: Implement"), "{files}");
        assert!(
            files.contains("file.txt") && files.contains("new.txt"),
            "{files}"
        );
        // A second call finds nothing left.
        assert_eq!(
            manager.commit_changes(&wt, "task", "again").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn commit_changes_refuses_another_branch() {
        let dir = tempdir().unwrap();
        let (_repo, wt) = repo_with_worktree(dir.path());
        std::fs::write(wt.join("x.txt"), "x").unwrap();
        let err = WorkspaceManager::new()
            .commit_changes(&wt, "main", "m")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "conflict", "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn repository_hooks_never_run_for_commit_or_push() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let (repo, wt, _origin) = repo_with_origin(dir.path());
        let marker = dir.path().join("hook-ran");
        for hook in ["pre-commit", "commit-msg", "post-commit", "pre-push"] {
            let path = repo.join(".git").join("hooks").join(hook);
            std::fs::write(&path, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(wt.join("new.txt"), "new\n").unwrap();
        let manager = WorkspaceManager::new();
        manager
            .commit_changes(&wt, "task", "msg")
            .await
            .unwrap()
            .unwrap();
        manager.push_branch(&wt, "task", None).await.unwrap();
        assert!(!marker.exists(), "a repository hook was executed");
        // Sanity check of the test itself: the same hook does run for a plain git commit.
        std::fs::write(wt.join("plain.txt"), "p\n").unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["commit", "-m", "plain"]);
        assert!(marker.exists(), "the test hook is not effective");
    }

    #[tokio::test]
    async fn push_branch_publishes_the_branch_and_never_forces() {
        let dir = tempdir().unwrap();
        let (_repo, wt, origin) = repo_with_origin(dir.path());
        std::fs::write(wt.join("new.txt"), "new\n").unwrap();
        let manager = WorkspaceManager::new();
        let sha = manager
            .commit_changes(&wt, "task", "msg")
            .await
            .unwrap()
            .unwrap();
        manager.push_branch(&wt, "task", None).await.unwrap();
        let remote_head = git_stdout(&origin, &["rev-parse", "refs/heads/task"])
            .await
            .unwrap();
        assert_eq!(remote_head.trim(), sha);

        // The remote branch diverges (someone else pushed): the push is refused, not forced.
        let other = dir.path().join("other");
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        git(&other, &["checkout", "-q", "task"]);
        std::fs::write(other.join("theirs.txt"), "theirs\n").unwrap();
        git(&other, &["add", "-A"]);
        git(&other, &["commit", "-m", "theirs"]);
        git(&other, &["push", "-q", "origin", "task"]);
        std::fs::write(wt.join("mine.txt"), "mine\n").unwrap();
        manager
            .commit_changes(&wt, "task", "mine")
            .await
            .unwrap()
            .unwrap();
        let err = manager.push_branch(&wt, "task", None).await.unwrap_err();
        assert_eq!(err.code(), "git_worktree_error", "{err}");
        assert!(manager.push_branch(&wt, "--delete", None).await.is_err());
    }

    #[tokio::test]
    async fn a_failed_authenticated_push_never_echoes_the_token() {
        let dir = tempdir().unwrap();
        let (repo, wt) = repo_with_worktree(dir.path());
        // An https origin nothing listens on: the push fails fast, over the authenticated path.
        git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://127.0.0.1:1/acme/widgets.git",
            ],
        );
        std::fs::write(wt.join("new.txt"), "new\n").unwrap();
        let manager = WorkspaceManager::new();
        manager
            .commit_changes(&wt, "task", "msg")
            .await
            .unwrap()
            .unwrap();
        let token = "ghp_secrettokensecrettokensecrettoken12";
        let err = manager
            .push_branch(&wt, "task", Some(token))
            .await
            .unwrap_err();
        assert!(!err.to_string().contains(token), "{err}");
        assert_eq!(
            https_origin("https://github.com/acme/widgets.git").as_deref(),
            Some("https://github.com/")
        );
        assert_eq!(https_origin("git@github.com:acme/widgets.git"), None);
        assert_eq!(https_origin("/local/path.git"), None);
    }
}
