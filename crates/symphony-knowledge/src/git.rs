//! Read-only git access for the Knowledge Base.
//!
//! Every call goes through `symphony_workspaces::git_command` (no console window on Windows) and
//! `output_with_timeout` (the whole process tree ends on a timeout). The commands are hardened for
//! observing a repository other tools are working in:
//!
//! - `GIT_OPTIONAL_LOCKS=0`: `git status` must not take `index.lock` away from an agent that is
//!   about to commit.
//! - `core.fsmonitor=false`, `--no-ext-diff`, `--no-textconv`: a repository's configuration can
//!   name programs to run for these; reading history must never execute them.
//! - Secret environment variables are not inherited.
//! - Revisions are 7-64 lowercase hex digits (validated), paths come after `--`.

use crate::model::{ChangedFile, CommitSummary, DiffHunk, FileChangeStatus, WorktreeKind};
use chrono::{DateTime, TimeZone, Utc};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use symphony_core::{SymphonyError, SymphonyResult};
use symphony_workspaces::{
    git_command, output_with_timeout, prepare_tree_command, ProcessTree, SECRET_ENV_VARS_TO_STRIP,
};
use tokio::io::AsyncReadExt;

const GIT_TIMEOUT: Duration = Duration::from_secs(60);
/// Files listed for one commit; the rest is reported as truncated.
pub const MAX_FILES_PER_COMMIT: usize = 500;
const MAX_HUNKS_PER_FILE: usize = 200;

/// `true` for a lowercase hex object name of a plausible length (abbreviated or full).
pub fn is_hex_rev(value: &str) -> bool {
    (7..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Reject anything that is not a lowercase hex revision. Used on every value that comes from a
/// client before it reaches a git command line.
pub fn validate_rev(value: &str) -> SymphonyResult<&str> {
    if is_hex_rev(value) {
        Ok(value)
    } else {
        Err(SymphonyError::InvalidConfig(format!(
            "`{value}` is not a commit id (7 to 64 lowercase hex digits)"
        )))
    }
}

pub fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// Stable identifier of a worktree, derived from its normalised path.
pub fn worktree_id(path: &str) -> String {
    let digest = Sha256::digest(normalize_path_key(path).as_bytes());
    format!("wt_{}", &hex::encode(digest)[..12])
}

/// A comparable form of a filesystem path: forward slashes, no trailing slash, no Windows
/// verbatim prefix, lower-cased on Windows (case-insensitive file systems).
pub fn normalize_path_key(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    if let Some(rest) = p.strip_prefix("//?/") {
        p = rest.to_string();
    }
    while p.len() > 1 && p.ends_with('/') {
        p.pop();
    }
    if cfg!(windows) {
        p = p.to_lowercase();
    }
    p
}

/// Like [`normalize_path_key`], but resolved first when the path exists, for matching a path from
/// one source with the same directory from another: git prints a worktree's long, canonical path
/// while a path built by Symphony (or a temp directory) can be a Windows 8.3 short name
/// (`RUNNER~1`) or go through a link. Not used for ids, which must not change when a directory goes.
pub fn path_match_key(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(resolved) => normalize_path_key(&resolved.to_string_lossy()),
        Err(_) => normalize_path_key(path),
    }
}

fn git_error(args: &[&str], detail: &str) -> SymphonyError {
    let verb = args.first().copied().unwrap_or("git");
    SymphonyError::GitWorktreeError(format!("git {verb}: {}", detail.trim()))
}

fn base_command(cwd: &Path) -> tokio::process::Command {
    let mut cmd = git_command();
    cmd.arg("-C")
        .arg(cwd)
        .args([
            "-c",
            "core.quotePath=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "log.showSignature=false",
        ])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for var in SECRET_ENV_VARS_TO_STRIP {
        cmd.env_remove(var);
    }
    cmd
}

/// Run git in `cwd`; returns stdout and the exit code (-1 when killed by a signal).
async fn run_git_status(cwd: &Path, args: &[&str]) -> SymphonyResult<(Vec<u8>, i32)> {
    let mut cmd = base_command(cwd);
    cmd.args(args);
    match output_with_timeout(&mut cmd, GIT_TIMEOUT).await {
        None => Err(git_error(args, "timed out")),
        Some(Err(e)) => Err(git_error(args, &e.to_string())),
        Some(Ok(out)) => Ok((out.stdout, out.status.code().unwrap_or(-1))),
    }
}

/// Run git in `cwd` and require success.
async fn run_git(cwd: &Path, args: &[&str]) -> SymphonyResult<Vec<u8>> {
    let mut cmd = base_command(cwd);
    cmd.args(args);
    match output_with_timeout(&mut cmd, GIT_TIMEOUT).await {
        None => Err(git_error(args, "timed out")),
        Some(Err(e)) => Err(git_error(args, &e.to_string())),
        Some(Ok(out)) if out.status.success() => Ok(out.stdout),
        Some(Ok(out)) => Err(git_error(args, &String::from_utf8_lossy(&out.stderr))),
    }
}

async fn run_git_text(cwd: &Path, args: &[&str]) -> SymphonyResult<String> {
    Ok(String::from_utf8_lossy(&run_git(cwd, args).await?).into_owned())
}

/// Run git and read at most `limit` bytes of stdout; the process tree is killed once the budget
/// is spent. Returns the bytes and whether output was cut off.
async fn run_git_bounded(
    cwd: &Path,
    args: &[&str],
    limit: usize,
) -> SymphonyResult<(Vec<u8>, bool)> {
    let mut cmd = base_command(cwd);
    cmd.args(args).stderr(Stdio::null());
    prepare_tree_command(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| git_error(args, &e.to_string()))?;
    let mut tree = ProcessTree::attach(&child);
    let Some(stdout) = child.stdout.take() else {
        tree.kill();
        return Err(git_error(args, "no stdout"));
    };
    let mut buf = Vec::new();
    let mut limited = stdout.take(limit as u64 + 1);
    let read = tokio::time::timeout(GIT_TIMEOUT, limited.read_to_end(&mut buf)).await;
    let truncated = buf.len() > limit;
    buf.truncate(limit);
    tree.kill();
    let _ = child.wait().await;
    match read {
        Ok(Ok(_)) => Ok((buf, truncated)),
        Ok(Err(e)) => Err(git_error(args, &e.to_string())),
        Err(_) => Err(git_error(args, "timed out")),
    }
}

fn split_nul(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

fn ts(seconds: &str) -> DateTime<Utc> {
    seconds
        .trim()
        .parse::<i64>()
        .ok()
        .and_then(|s| Utc.timestamp_opt(s, 0).single())
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default())
}

// ---------------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------------

/// A worktree as `git worktree list --porcelain` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitWorktree {
    pub path: String,
    pub head: Option<String>,
    /// Short branch name (`refs/heads/` stripped); `None` when detached.
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
    /// The first entry is always the main worktree.
    pub kind: WorktreeKind,
}

/// A commit as `git log` reports it (no worktree association yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCommit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub authored_at: DateTime<Utc>,
    pub committed_at: DateTime<Utc>,
    pub subject: String,
}

impl RawCommit {
    pub fn into_summary(self) -> CommitSummary {
        CommitSummary {
            short_sha: short_sha(&self.sha),
            is_merge: self.parents.len() > 1,
            sha: self.sha,
            parents: self.parents,
            author_name: self.author_name,
            authored_at: self.authored_at,
            committed_at: self.committed_at,
            subject: self.subject,
            on_default_branch: false,
            unpushed: None,
            refs: Vec::new(),
            worktrees: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub mode: String,
    pub object: String,
    pub size: Option<u64>,
    pub path: String,
}

// ---------------------------------------------------------------------------------------------
// Repository handle
// ---------------------------------------------------------------------------------------------

/// Read access to one repository, addressed by any of its worktrees (they share one object
/// database, so a single handle sees every commit).
#[derive(Debug, Clone)]
pub struct GitRepo {
    root: PathBuf,
}

impl GitRepo {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory shared by all worktrees (`.git` of the main checkout).
    pub async fn common_dir(&self) -> SymphonyResult<PathBuf> {
        let text = run_git_text(
            &self.root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await?;
        let dir = PathBuf::from(text.trim());
        if dir.is_absolute() {
            Ok(dir)
        } else {
            Ok(self.root.join(dir))
        }
    }

    /// Resolve a hex revision (possibly abbreviated) to a full commit id.
    pub async fn resolve_commit(&self, rev: &str) -> SymphonyResult<String> {
        validate_rev(rev)?;
        let spec = format!("{rev}^{{commit}}");
        let (out, code) =
            run_git_status(&self.root, &["rev-parse", "--verify", "--quiet", &spec]).await?;
        let sha = String::from_utf8_lossy(&out).trim().to_string();
        if code != 0 || sha.is_empty() {
            return Err(SymphonyError::NotFound(format!("commit {rev} not found")));
        }
        Ok(sha)
    }

    /// The commit a local or remote-tracking branch points at.
    pub async fn branch_tip(&self, branch: &str) -> Option<String> {
        if branch.is_empty()
            || branch.starts_with('-')
            || branch.contains(['\0', '\n', ' ', '~', '^', ':', '?', '*', '[', '\\'])
        {
            return None;
        }
        for candidate in [
            format!("refs/heads/{branch}^{{commit}}"),
            format!("refs/remotes/origin/{branch}^{{commit}}"),
        ] {
            if let Ok((out, 0)) = run_git_status(
                &self.root,
                &["rev-parse", "--verify", "--quiet", &candidate],
            )
            .await
            {
                let sha = String::from_utf8_lossy(&out).trim().to_string();
                if !sha.is_empty() {
                    return Some(sha);
                }
            }
        }
        None
    }

    /// Every worktree of the repository, main one first.
    pub async fn worktrees(&self) -> SymphonyResult<Vec<GitWorktree>> {
        let bytes = match run_git(&self.root, &["worktree", "list", "--porcelain", "-z"]).await {
            Ok(bytes) => bytes,
            // `-z` needs git 2.36; older versions are parsed line by line.
            Err(_) => {
                let text = run_git_text(&self.root, &["worktree", "list", "--porcelain"]).await?;
                return Ok(parse_worktree_list(
                    &text.lines().map(str::to_string).collect::<Vec<_>>(),
                ));
            }
        };
        Ok(parse_worktree_list(&split_nul(&bytes)))
    }

    /// Commits reachable from `revs` (full ids), newest first in date order, at most `limit`.
    /// The result is a prefix of a topological order, so every commit between a head and any
    /// listed ancestor is listed too (which `reachability` relies on).
    pub async fn log(&self, revs: &[String], limit: usize) -> SymphonyResult<Vec<RawCommit>> {
        if revs.is_empty() {
            return Ok(Vec::new());
        }
        for rev in revs {
            validate_rev(rev)?;
        }
        let limit_arg = limit.to_string();
        let mut args: Vec<&str> = vec![
            "log",
            "--date-order",
            "--no-color",
            "-n",
            &limit_arg,
            "--format=%H%x1f%P%x1f%an%x1f%at%x1f%ct%x1f%s%x1e",
        ];
        args.extend(revs.iter().map(String::as_str));
        args.push("--");
        let text = run_git_text(&self.root, &args).await?;
        Ok(parse_log(&text))
    }

    /// One commit (no walking), for a worktree head outside the listed window.
    pub async fn commit(&self, sha: &str) -> SymphonyResult<RawCommit> {
        validate_rev(sha)?;
        let text = run_git_text(
            &self.root,
            &[
                "log",
                "--no-walk",
                "--no-color",
                "--format=%H%x1f%P%x1f%an%x1f%at%x1f%ct%x1f%s%x1e",
                sha,
                "--",
            ],
        )
        .await?;
        parse_log(&text)
            .into_iter()
            .next()
            .ok_or_else(|| SymphonyError::NotFound(format!("commit {sha} not found")))
    }

    /// Branch / tag names by the commit they point at (annotated tags resolve to their commit).
    pub async fn refs_by_commit(&self) -> SymphonyResult<HashMap<String, Vec<String>>> {
        let text = run_git_text(
            &self.root,
            &[
                "for-each-ref",
                "--format=%(objectname) %(*objectname) %(refname)",
                "refs/heads",
                "refs/remotes",
                "refs/tags",
            ],
        )
        .await?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for line in text.lines() {
            // "<object> <peeled object or empty> <refname>"; the empty field leaves two spaces.
            let mut parts = line.splitn(3, ' ');
            let (Some(object), Some(peeled), Some(name)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let label = if let Some(branch) = name.strip_prefix("refs/heads/") {
                branch.to_string()
            } else if let Some(remote) = name.strip_prefix("refs/remotes/") {
                if remote.ends_with("/HEAD") {
                    continue;
                }
                remote.to_string()
            } else if let Some(tag) = name.strip_prefix("refs/tags/") {
                format!("tag: {tag}")
            } else {
                continue;
            };
            let target = if peeled.is_empty() { object } else { peeled };
            map.entry(target.to_string()).or_default().push(label);
        }
        for names in map.values_mut() {
            names.sort();
        }
        Ok(map)
    }

    /// Whether any remote-tracking ref exists (otherwise "unpushed" says nothing).
    pub async fn has_remote_refs(&self) -> bool {
        match run_git_text(
            &self.root,
            &[
                "for-each-ref",
                "--count=1",
                "--format=%(refname)",
                "refs/remotes",
            ],
        )
        .await
        {
            Ok(text) => !text.trim().is_empty(),
            Err(_) => false,
        }
    }

    /// Commits reachable from `heads` that no remote-tracking ref contains.
    pub async fn unpushed(
        &self,
        heads: &[String],
        limit: usize,
    ) -> SymphonyResult<HashSet<String>> {
        if heads.is_empty() {
            return Ok(HashSet::new());
        }
        for head in heads {
            validate_rev(head)?;
        }
        let limit_arg = limit.to_string();
        let mut args: Vec<&str> = vec!["rev-list", "-n", &limit_arg];
        args.extend(heads.iter().map(String::as_str));
        args.extend(["--not", "--remotes", "--"]);
        let text = run_git_text(&self.root, &args).await?;
        Ok(text.lines().map(|l| l.trim().to_string()).collect())
    }

    /// `ancestor` is reachable from `descendant` (a commit counts as its own ancestor).
    pub async fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        if validate_rev(ancestor).is_err() || validate_rev(descendant).is_err() {
            return false;
        }
        matches!(
            run_git_status(
                &self.root,
                &["merge-base", "--is-ancestor", ancestor, descendant]
            )
            .await,
            Ok((_, 0))
        )
    }

    /// No remote-tracking ref contains `sha`. Only meaningful when remote refs exist.
    pub async fn is_unpushed(&self, sha: &str) -> bool {
        if validate_rev(sha).is_err() {
            return false;
        }
        match run_git_text(
            &self.root,
            &["rev-list", "-n", "1", sha, "--not", "--remotes", "--"],
        )
        .await
        {
            Ok(text) => !text.trim().is_empty(),
            Err(_) => false,
        }
    }

    /// Commits whose message mentions any of `terms` (case-insensitive, literal), newest first.
    pub async fn search_commits(
        &self,
        tip: &str,
        terms: &[String],
        limit: usize,
    ) -> SymphonyResult<Vec<RawCommit>> {
        validate_rev(tip)?;
        let usable: Vec<&String> = terms
            .iter()
            .filter(|t| !t.is_empty() && !t.starts_with('-') && !t.contains(['\0', '\n']))
            .collect();
        if usable.is_empty() {
            return Ok(Vec::new());
        }
        let limit_arg = limit.to_string();
        let greps: Vec<String> = usable.iter().map(|t| format!("--grep={t}")).collect();
        let mut args: Vec<&str> = vec![
            "log",
            "--no-color",
            "--date-order",
            "--regexp-ignore-case",
            "--fixed-strings",
            "-n",
            &limit_arg,
            "--format=%H%x1f%P%x1f%an%x1f%at%x1f%ct%x1f%s%x1e",
        ];
        args.extend(greps.iter().map(String::as_str));
        args.push(tip);
        args.push("--");
        Ok(parse_log(&run_git_text(&self.root, &args).await?))
    }

    /// Number of commits on `head` that `base` does not contain.
    pub async fn ahead_count(&self, head: &str, base: &str) -> Option<u32> {
        validate_rev(head).ok()?;
        validate_rev(base).ok()?;
        let range = format!("{base}..{head}");
        let text = run_git_text(&self.root, &["rev-list", "--count", &range, "--"])
            .await
            .ok()?;
        text.trim().parse().ok()
    }

    /// Uncommitted paths of a worktree (`git status`), untracked files included.
    pub async fn status_paths(worktree: &Path) -> Vec<String> {
        let Ok(bytes) = run_git(
            worktree,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--no-renames",
                "--untracked-files=normal",
            ],
        )
        .await
        else {
            return Vec::new();
        };
        split_nul(&bytes)
            .into_iter()
            .filter(|entry| entry.len() > 3)
            .map(|entry| entry[3..].to_string())
            .collect()
    }

    /// Commits that changed `path`, newest first.
    pub async fn log_for_path(
        &self,
        sha: &str,
        path: &str,
        limit: usize,
    ) -> SymphonyResult<Vec<RawCommit>> {
        validate_rev(sha)?;
        let limit_arg = limit.to_string();
        let text = run_git_text(
            &self.root,
            &[
                "log",
                "--no-color",
                "--date-order",
                "-n",
                &limit_arg,
                "--format=%H%x1f%P%x1f%an%x1f%at%x1f%ct%x1f%s%x1e",
                sha,
                "--",
                path,
            ],
        )
        .await?;
        Ok(parse_log(&text))
    }

    async fn empty_tree(&self) -> SymphonyResult<String> {
        // The empty tree's id depends on the repository's hash algorithm.
        let mut cmd = base_command(&self.root);
        cmd.args(["hash-object", "-t", "tree", "--stdin"]);
        match output_with_timeout(&mut cmd, GIT_TIMEOUT).await {
            Some(Ok(out)) if out.status.success() => {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            }
            _ => Ok("4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()),
        }
    }

    /// Details of one commit: parents, message body and changed files with their line ranges,
    /// relative to the first parent (the empty tree for a root commit).
    pub async fn commit_detail(&self, sha: &str) -> SymphonyResult<RawDetail> {
        let sha = self.resolve_commit(sha).await?;
        let commit = self.commit(&sha).await?;
        let body = run_git_text(
            &self.root,
            &["log", "--no-walk", "--no-color", "--format=%b", &sha, "--"],
        )
        .await?
        .trim()
        .to_string();
        let base = match commit.parents.first() {
            Some(parent) => parent.clone(),
            None => self.empty_tree().await?,
        };
        let diff_base = commit.parents.first().cloned();

        let name_status = run_git(
            &self.root,
            &[
                "diff",
                "--name-status",
                "-z",
                "--find-renames",
                "--no-ext-diff",
                "--no-textconv",
                &base,
                &sha,
                "--",
            ],
        )
        .await?;
        let numstat = run_git(
            &self.root,
            &[
                "diff",
                "--numstat",
                "-z",
                "--find-renames",
                "--no-ext-diff",
                "--no-textconv",
                &base,
                &sha,
                "--",
            ],
        )
        .await?;
        let (patch, patch_truncated) = run_git_bounded(
            &self.root,
            &[
                "diff",
                "-U0",
                "--no-color",
                "--find-renames",
                "--no-ext-diff",
                "--no-textconv",
                "--no-prefix",
                &base,
                &sha,
                "--",
            ],
            8 * 1024 * 1024,
        )
        .await?;

        let mut files = parse_name_status(&split_nul(&name_status));
        let counts = parse_numstat(&split_nul(&numstat));
        let hunks = parse_patch_hunks(&String::from_utf8_lossy(&patch));
        let mut additions = 0u32;
        let mut deletions = 0u32;
        for file in &mut files {
            if let Some((add, del, binary)) = counts.get(&file.path) {
                file.additions = *add;
                file.deletions = *del;
                file.binary = *binary;
                additions += add.unwrap_or(0);
                deletions += del.unwrap_or(0);
            }
            if let Some(file_hunks) = hunks
                .get(&file.path)
                .or_else(|| file.old_path.as_ref().and_then(|p| hunks.get(p)))
            {
                file.hunks = file_hunks.clone();
            }
        }
        let files_truncated = files.len() > MAX_FILES_PER_COMMIT;
        files.truncate(MAX_FILES_PER_COMMIT);
        Ok(RawDetail {
            commit,
            body,
            files,
            additions,
            deletions,
            files_truncated,
            patch_truncated,
            diff_base,
        })
    }

    /// The unified diff of a commit against its first parent, at most `max_bytes`.
    pub async fn diff_text(
        &self,
        sha: &str,
        context: u32,
        max_bytes: usize,
    ) -> SymphonyResult<(String, bool)> {
        let sha = self.resolve_commit(sha).await?;
        let commit = self.commit(&sha).await?;
        let base = match commit.parents.first() {
            Some(parent) => parent.clone(),
            None => self.empty_tree().await?,
        };
        let context_arg = format!("-U{}", context.min(20));
        let (bytes, truncated) = run_git_bounded(
            &self.root,
            &[
                "diff",
                &context_arg,
                "--no-color",
                "--find-renames",
                "--no-ext-diff",
                "--no-textconv",
                &base,
                &sha,
                "--",
            ],
            max_bytes,
        )
        .await?;
        Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
    }

    /// Recursive listing of a commit's tree with blob sizes.
    pub async fn ls_tree(&self, sha: &str) -> SymphonyResult<Vec<TreeEntry>> {
        validate_rev(sha)?;
        let bytes = run_git(
            &self.root,
            &["ls-tree", "-r", "-z", "--long", "--full-tree", sha, "--"],
        )
        .await?;
        Ok(parse_ls_tree(&split_nul(&bytes)))
    }

    /// The blob at `<sha>:<path>`, at most `max_bytes`; `None` when the path does not exist there.
    pub async fn blob(
        &self,
        sha: &str,
        path: &str,
        max_bytes: usize,
    ) -> SymphonyResult<Option<Vec<u8>>> {
        validate_rev(sha)?;
        // The spec below starts with the commit id, so a path beginning with `-` is no option.
        if path.is_empty() || path.contains('\0') {
            return Err(SymphonyError::InvalidConfig(format!(
                "`{path}` is not a repository path"
            )));
        }
        let spec = format!("{sha}:{path}");
        let exists = run_git_status(&self.root, &["cat-file", "-e", &spec])
            .await?
            .1
            == 0;
        if !exists {
            return Ok(None);
        }
        let (bytes, _) =
            run_git_bounded(&self.root, &["cat-file", "blob", &spec], max_bytes).await?;
        Ok(Some(bytes))
    }
}

/// [`GitRepo::commit_detail`] before the worktree/ref decoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDetail {
    pub commit: RawCommit,
    pub body: String,
    pub files: Vec<ChangedFile>,
    pub additions: u32,
    pub deletions: u32,
    pub files_truncated: bool,
    /// The hunk data was cut at its size limit: files late in the commit have no hunks.
    pub patch_truncated: bool,
    pub diff_base: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Parsers (pure, unit-tested)
// ---------------------------------------------------------------------------------------------

/// Parse `git worktree list --porcelain` fields. `-z` output is a flat list of fields with an
/// empty field between records; the line-based form has a blank line between records, so both
/// are handled by treating an empty entry as a record boundary.
pub fn parse_worktree_list(fields: &[String]) -> Vec<GitWorktree> {
    let mut out: Vec<GitWorktree> = Vec::new();
    let mut current: Option<GitWorktree> = None;
    let flush = |current: &mut Option<GitWorktree>, out: &mut Vec<GitWorktree>| {
        if let Some(mut wt) = current.take() {
            wt.kind = if out.is_empty() {
                WorktreeKind::Main
            } else {
                WorktreeKind::External
            };
            out.push(wt);
        }
    };
    for field in fields {
        if field.is_empty() {
            flush(&mut current, &mut out);
            continue;
        }
        if let Some(path) = field.strip_prefix("worktree ") {
            flush(&mut current, &mut out);
            current = Some(GitWorktree {
                path: path.to_string(),
                head: None,
                branch: None,
                detached: false,
                bare: false,
                locked: false,
                prunable: false,
                kind: WorktreeKind::External,
            });
            continue;
        }
        let Some(wt) = current.as_mut() else { continue };
        if let Some(head) = field.strip_prefix("HEAD ") {
            // An unborn branch reports an all-zero id.
            if head.bytes().any(|b| b != b'0') {
                wt.head = Some(head.to_string());
            }
        } else if let Some(branch) = field.strip_prefix("branch ") {
            wt.branch = Some(
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch)
                    .to_string(),
            );
        } else if field == "detached" {
            wt.detached = true;
        } else if field == "bare" {
            wt.bare = true;
        } else if field == "locked" || field.starts_with("locked ") {
            wt.locked = true;
        } else if field == "prunable" || field.starts_with("prunable ") {
            wt.prunable = true;
        }
    }
    flush(&mut current, &mut out);
    out
}

pub fn parse_log(text: &str) -> Vec<RawCommit> {
    text.split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim_matches(|c| c == '\n' || c == '\r');
            if record.is_empty() {
                return None;
            }
            let mut fields = record.split('\u{1f}');
            let sha = fields.next()?.trim().to_string();
            if !is_hex_rev(&sha) {
                return None;
            }
            let parents = fields
                .next()?
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let author_name = fields.next()?.to_string();
            let authored_at = ts(fields.next()?);
            let committed_at = ts(fields.next()?);
            let subject = fields.next().unwrap_or("").to_string();
            Some(RawCommit {
                sha,
                parents,
                author_name,
                authored_at,
                committed_at,
                subject,
            })
        })
        .collect()
}

pub fn parse_ls_tree(fields: &[String]) -> Vec<TreeEntry> {
    fields
        .iter()
        .filter_map(|entry| {
            // "<mode> <type> <object> <size>\t<path>", the size right-aligned with spaces.
            let (meta, path) = entry.split_once('\t')?;
            let mut parts = meta.split_whitespace();
            let mode = parts.next()?.to_string();
            let kind = parts.next()?;
            if kind != "blob" && kind != "commit" {
                return None;
            }
            let object = parts.next()?.to_string();
            let size = parts.next().and_then(|s| s.parse::<u64>().ok());
            Some(TreeEntry {
                mode,
                object,
                size,
                path: path.to_string(),
            })
        })
        .collect()
}

fn status_of(letter: &str) -> Option<FileChangeStatus> {
    match letter.chars().next()? {
        'A' => Some(FileChangeStatus::Added),
        'M' => Some(FileChangeStatus::Modified),
        'D' => Some(FileChangeStatus::Deleted),
        'R' => Some(FileChangeStatus::Renamed),
        'C' => Some(FileChangeStatus::Copied),
        'T' => Some(FileChangeStatus::TypeChanged),
        _ => None,
    }
}

/// Parse `git diff --name-status -z`: `M\0path\0`, renames `R100\0old\0new\0`.
pub fn parse_name_status(fields: &[String]) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let code = fields[i].as_str();
        i += 1;
        if code.is_empty() {
            continue;
        }
        let Some(status) = status_of(code) else {
            continue;
        };
        let renamed = matches!(status, FileChangeStatus::Renamed | FileChangeStatus::Copied);
        if renamed {
            let (Some(old), Some(new)) = (fields.get(i), fields.get(i + 1)) else {
                break;
            };
            files.push(ChangedFile {
                path: new.clone(),
                old_path: Some(old.clone()),
                status,
                additions: None,
                deletions: None,
                binary: false,
                hunks: Vec::new(),
            });
            i += 2;
        } else {
            let Some(path) = fields.get(i) else { break };
            files.push(ChangedFile {
                path: path.clone(),
                old_path: None,
                status,
                additions: None,
                deletions: None,
                binary: false,
                hunks: Vec::new(),
            });
            i += 1;
        }
    }
    files
}

/// Parse `git diff --numstat -z`: `add\tdel\tpath\0`, renames `add\tdel\t\0old\0new\0`, binary
/// files report `-\t-`. Keyed by the new path.
pub fn parse_numstat(fields: &[String]) -> HashMap<String, (Option<u32>, Option<u32>, bool)> {
    let mut out = HashMap::new();
    let mut i = 0;
    while i < fields.len() {
        let entry = &fields[i];
        i += 1;
        if entry.is_empty() {
            continue;
        }
        let mut parts = entry.splitn(3, '\t');
        let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let binary = add == "-" && del == "-";
        let counts = (add.parse().ok(), del.parse().ok(), binary);
        if path.is_empty() {
            // Rename: the next two fields are the old and the new path.
            if let Some(new) = fields.get(i + 1) {
                out.insert(new.clone(), counts);
            }
            i += 2;
        } else {
            out.insert(path.to_string(), counts);
        }
    }
    out
}

fn unquote_git_path(raw: &str) -> String {
    let raw = raw.trim_end_matches('\t');
    let Some(inner) = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return raw.to_string();
    };
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        match bytes[i] {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            d @ b'0'..=b'7' => {
                let mut value = (d - b'0') as u32;
                let mut digits = 1;
                while digits < 3 && i + 1 < bytes.len() && (b'0'..=b'7').contains(&bytes[i + 1]) {
                    i += 1;
                    value = value * 8 + (bytes[i] - b'0') as u32;
                    digits += 1;
                }
                out.push(value as u8);
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `@@ -a[,b] +c[,d] @@` -> hunk.
pub fn parse_hunk_header(line: &str) -> Option<DiffHunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let range = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (old_start, old_lines) = range(old)?;
    let (new_start, new_lines) = range(new)?;
    Some(DiffHunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
    })
}

/// Hunks of a `-U0 --no-prefix` patch, keyed by the new path (the old path for deletions).
pub fn parse_patch_hunks(patch: &str) -> HashMap<String, Vec<DiffHunk>> {
    let mut out: HashMap<String, Vec<DiffHunk>> = HashMap::new();
    let mut old_path: Option<String> = None;
    let mut key: Option<String> = None;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            old_path = None;
            key = None;
        } else if let Some(rest) = line.strip_prefix("--- ") {
            old_path = (rest != "/dev/null").then(|| unquote_git_path(rest));
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            let new_path = (rest != "/dev/null").then(|| unquote_git_path(rest));
            key = new_path.or_else(|| old_path.clone());
        } else if line.starts_with("@@ ") {
            if let (Some(k), Some(hunk)) = (key.as_ref(), parse_hunk_header(line)) {
                let hunks = out.entry(k.clone()).or_default();
                if hunks.len() < MAX_HUNKS_PER_FILE {
                    hunks.push(hunk);
                }
            }
        }
    }
    out
}

/// Which listed commits are reachable from `head` by following parent links *inside* the listed
/// set. Exact for a prefix of a topological order (see [`GitRepo::log`]).
pub fn reachable_within(head: &str, parents: &HashMap<&str, &[String]>) -> HashSet<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack = vec![head.to_string()];
    while let Some(sha) = stack.pop() {
        if !parents.contains_key(sha.as_str()) || !seen.insert(sha.clone()) {
            continue;
        }
        if let Some(ps) = parents.get(sha.as_str()) {
            stack.extend(ps.iter().cloned());
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_to_one_directory_match_however_they_are_spelled() {
        let temp = tempfile::TempDir::new().unwrap();
        let wt = temp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(temp.path().join("other")).unwrap();
        let roundabout = temp.path().join("other").join("..").join("wt");
        let plain = wt.to_string_lossy().into_owned();
        let indirect = roundabout.to_string_lossy().into_owned();
        // A spelling with `..` (or, on Windows, an 8.3 short name) is the same directory...
        assert_eq!(path_match_key(&plain), path_match_key(&indirect));
        // ...a directory that is gone falls back to the plain normalised form.
        let gone = temp.path().join("gone").to_string_lossy().into_owned();
        assert_eq!(path_match_key(&gone), normalize_path_key(&gone));
    }

    #[test]
    fn worktree_list_parses_main_linked_detached_and_flags() {
        let text = "worktree /repo\nHEAD 1111111111111111111111111111111111111111\nbranch refs/heads/main\n\nworktree /repo/.wt/a\nHEAD 2222222222222222222222222222222222222222\ndetached\nlocked reason here\n\nworktree /repo/.wt/gone\nHEAD 3333333333333333333333333333333333333333\nbranch refs/heads/feat/x\nprunable gitdir file points to non-existent location\n";
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let list = parse_worktree_list(&lines);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].kind, WorktreeKind::Main);
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(list[1].kind, WorktreeKind::External);
        assert!(list[1].detached && list[1].locked && list[1].branch.is_none());
        assert_eq!(list[2].branch.as_deref(), Some("feat/x"));
        assert!(list[2].prunable);
    }

    #[test]
    fn nul_separated_worktree_list_parses_like_the_line_form() {
        let raw = "worktree /repo\0HEAD 1111111111111111111111111111111111111111\0branch refs/heads/main\0\0worktree /repo/wt\0HEAD 0000000000000000000000000000000000000000\0branch refs/heads/new\0\0";
        let list = parse_worktree_list(&split_nul(raw.as_bytes()));
        assert_eq!(list.len(), 2);
        // An unborn branch has no head commit yet.
        assert!(list[1].head.is_none());
        assert_eq!(list[1].branch.as_deref(), Some("new"));
    }

    #[test]
    fn log_records_parse_and_ignore_garbage() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let text = format!(
            "{a}\u{1f}{b}\u{1f}Ada\u{1f}1700000000\u{1f}1700000100\u{1f}Fix thing\u{1e}\n{b}\u{1f}\u{1f}Bo\u{1f}1690000000\u{1f}1690000000\u{1f}Init\u{1e}\nnot a record"
        );
        let commits = parse_log(&text);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, vec![b.clone()]);
        assert_eq!(commits[0].subject, "Fix thing");
        assert!(commits[1].parents.is_empty());
    }

    #[test]
    fn hunk_headers_parse_with_and_without_counts() {
        assert_eq!(
            parse_hunk_header("@@ -10,3 +10,4 @@ fn foo()"),
            Some(DiffHunk {
                old_start: 10,
                old_lines: 3,
                new_start: 10,
                new_lines: 4
            })
        );
        assert_eq!(
            parse_hunk_header("@@ -5 +5 @@"),
            Some(DiffHunk {
                old_start: 5,
                old_lines: 1,
                new_start: 5,
                new_lines: 1
            })
        );
        assert_eq!(
            parse_hunk_header("@@ -7,2 +6,0 @@"),
            Some(DiffHunk {
                old_start: 7,
                old_lines: 2,
                new_start: 6,
                new_lines: 0
            })
        );
        assert!(parse_hunk_header("not a hunk").is_none());
    }

    #[test]
    fn patch_hunks_are_keyed_by_new_path_or_old_path_for_deletions() {
        let patch = "diff --git a.rs a.rs\nindex 1..2 100644\n--- a.rs\n+++ a.rs\n@@ -1,0 +2,2 @@\n+x\n+y\n@@ -9 +11 @@\n-z\n+w\ndiff --git gone.rs gone.rs\ndeleted file mode 100644\n--- gone.rs\n+++ /dev/null\n@@ -1,3 +0,0 @@\n-a\n-b\n-c\n";
        let hunks = parse_patch_hunks(patch);
        assert_eq!(hunks["a.rs"].len(), 2);
        assert_eq!(hunks["a.rs"][0].new_start, 2);
        assert_eq!(hunks["gone.rs"][0].old_lines, 3);
    }

    #[test]
    fn quoted_paths_are_unescaped() {
        assert_eq!(unquote_git_path("\"a\\tb\\\"c\\\\d\""), "a\tb\"c\\d");
        assert_eq!(unquote_git_path("plain/path.rs\t"), "plain/path.rs");
        // Octal escapes are UTF-8 bytes.
        assert_eq!(unquote_git_path("\"\\303\\245\""), "å");
    }

    #[test]
    fn name_status_and_numstat_handle_renames() {
        let ns = split_nul(b"M\0a.rs\0R100\0old.rs\0new.rs\0D\0gone.rs\0");
        let files = parse_name_status(&ns);
        assert_eq!(files.len(), 3);
        assert_eq!(files[1].status, FileChangeStatus::Renamed);
        assert_eq!(files[1].old_path.as_deref(), Some("old.rs"));
        let nums = parse_numstat(&split_nul(b"3\t1\ta.rs\0-\t-\t\0old.bin\0new.bin\0"));
        assert_eq!(nums["a.rs"], (Some(3), Some(1), false));
        assert_eq!(nums["new.bin"], (None, None, true));
    }

    #[test]
    fn revisions_are_validated_before_reaching_git() {
        assert!(is_hex_rev("abcdef1"));
        assert!(!is_hex_rev("abcde"));
        assert!(!is_hex_rev("ABCDEF1"));
        assert!(!is_hex_rev("abcdef1; rm -rf /"));
        assert!(!is_hex_rev("--upload-pack=x"));
        assert!(validate_rev("HEAD").is_err());
    }

    #[test]
    fn worktree_ids_ignore_slash_style_and_trailing_separators() {
        assert_eq!(worktree_id("C:\\a\\b\\"), worktree_id("C:/a/b"));
        assert_ne!(worktree_id("C:/a/b"), worktree_id("C:/a/c"));
    }

    #[test]
    fn reachability_follows_parents_inside_the_listed_commits_only() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let c = "c".repeat(40);
        let outside = "d".repeat(40);
        let pa = vec![b.clone()];
        let pb = vec![outside.clone()];
        let pc: Vec<String> = vec![];
        let mut parents: HashMap<&str, &[String]> = HashMap::new();
        parents.insert(&a, &pa);
        parents.insert(&b, &pb);
        parents.insert(&c, &pc);
        let reach = reachable_within(&a, &parents);
        assert!(reach.contains(&a) && reach.contains(&b));
        assert!(!reach.contains(&c) && !reach.contains(&outside));
    }
}
