//! Live worktrees and the unified commit timeline.
//!
//! The timeline is one `git log` over the HEADs of **every** worktree (they share an object
//! database), so each commit appears exactly once however many worktrees contain it. A commit's
//! worktree associations come from following parent links from each worktree's HEAD inside the
//! listed commits, which is exact because the listing is a prefix of a topological order.

use crate::git::{
    normalize_path_key, reachable_within, short_sha, worktree_id, GitRepo, GitWorktree, RawCommit,
};
use crate::model::{
    CommitSummary, CommitTimeline, CommitWorktreeRef, KnowledgeWorktree, WorkingTreeChange,
    WorktreeKind, WorktreeTaskLink,
};
use chrono::Utc;
use futures::future::join_all;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use symphony_core::SymphonyResult;

/// Files listed per uncommitted worktree in the timeline.
const WORKING_TREE_FILE_PREVIEW: usize = 20;

/// The repository a Knowledge Base view is about.
#[derive(Debug, Clone)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    pub repo_path: PathBuf,
    pub default_branch: String,
}

/// Symphony tasks by normalised worktree path ([`normalize_path_key`]).
pub type TaskLinks = HashMap<String, WorktreeTaskLink>;

struct Inspected {
    wt: GitWorktree,
    id: String,
    exists: bool,
    changed: Vec<String>,
}

async fn inspect(wt: GitWorktree) -> Inspected {
    let exists = Path::new(&wt.path).is_dir();
    let changed = if exists && !wt.bare {
        GitRepo::status_paths(Path::new(&wt.path)).await
    } else {
        Vec::new()
    };
    Inspected {
        id: worktree_id(&wt.path),
        wt,
        exists,
        changed,
    }
}

async fn inspected_worktrees(repo: &GitRepo) -> SymphonyResult<Vec<Inspected>> {
    let listed: Vec<GitWorktree> = repo
        .worktrees()
        .await?
        .into_iter()
        .filter(|w| !w.bare)
        .collect();
    Ok(join_all(listed.into_iter().map(inspect)).await)
}

/// Every worktree of the repository with its HEAD commit, dirty state and, where one exists, the
/// Symphony task that owns it.
pub async fn list_worktrees(
    project: &ProjectRef,
    links: &TaskLinks,
) -> SymphonyResult<Vec<KnowledgeWorktree>> {
    let repo = GitRepo::new(&project.repo_path);
    let inspected = inspected_worktrees(&repo).await?;
    let default_tip = repo.branch_tip(&project.default_branch).await;

    let heads: Vec<String> = {
        let mut seen = HashSet::new();
        inspected
            .iter()
            .filter_map(|i| i.wt.head.clone())
            .filter(|h| seen.insert(h.clone()))
            .collect()
    };
    let head_commits: HashMap<String, RawCommit> = if heads.is_empty() {
        HashMap::new()
    } else {
        join_all(heads.iter().map(|h| repo.commit(h)))
            .await
            .into_iter()
            .flatten()
            .map(|c| (c.sha.clone(), c))
            .collect()
    };
    let ahead: Vec<Option<u32>> = join_all(inspected.iter().map(|i| {
        let repo = repo.clone();
        let base = default_tip.clone();
        let head = i.wt.head.clone();
        async move {
            match (head, base) {
                (Some(head), Some(base)) => repo.ahead_count(&head, &base).await,
                _ => None,
            }
        }
    }))
    .await;

    Ok(inspected
        .into_iter()
        .zip(ahead)
        .map(|(i, ahead_of_default)| {
            let task = links.get(&normalize_path_key(&i.wt.path)).cloned();
            let kind = match (&i.wt.kind, &task) {
                (WorktreeKind::Main, _) => WorktreeKind::Main,
                (_, Some(_)) => WorktreeKind::Symphony,
                _ => WorktreeKind::External,
            };
            let head = i.wt.head.as_ref().and_then(|h| head_commits.get(h));
            KnowledgeWorktree {
                id: i.id,
                path: i.wt.path.clone(),
                kind,
                branch: i.wt.branch.clone(),
                detached: i.wt.detached,
                head_sha: i.wt.head.clone(),
                head_short_sha: i.wt.head.as_deref().map(short_sha),
                head_subject: head.map(|c| c.subject.clone()),
                head_committed_at: head.map(|c| c.committed_at),
                exists: i.exists,
                dirty: !i.changed.is_empty(),
                changed_files: i.changed.len() as u32,
                locked: i.wt.locked,
                prunable: i.wt.prunable,
                ahead_of_default,
                task,
            }
        })
        .collect())
}

/// The newest `limit` commits across all worktrees, each once, with the worktrees that contain it.
pub async fn build_timeline(project: &ProjectRef, limit: usize) -> SymphonyResult<CommitTimeline> {
    let repo = GitRepo::new(&project.repo_path);
    let inspected = inspected_worktrees(&repo).await?;
    let default_tip = repo.branch_tip(&project.default_branch).await;

    let mut revs: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for sha in inspected
        .iter()
        .filter_map(|i| i.wt.head.clone())
        .chain(default_tip.clone())
    {
        if seen.insert(sha.clone()) {
            revs.push(sha);
        }
    }

    let mut raw = repo.log(&revs, limit + 1).await?;
    let truncated = raw.len() > limit;
    raw.truncate(limit);
    // A worktree sitting on an old commit still shows up, at its place in time.
    let mut listed: HashSet<String> = raw.iter().map(|c| c.sha.clone()).collect();
    // These rows are not connected to the listed commits through parent links (the commits
    // between are not listed), so their worktrees and default-branch status are asked of git.
    let mut outside: Vec<String> = Vec::new();
    for head in &revs {
        if !listed.contains(head) {
            if let Ok(commit) = repo.commit(head).await {
                listed.insert(commit.sha.clone());
                outside.push(commit.sha.clone());
                raw.push(commit);
            }
        }
    }
    raw.sort_by_key(|c| std::cmp::Reverse(c.committed_at));

    let has_remote_refs = repo.has_remote_refs().await;
    let unpushed = if has_remote_refs {
        repo.unpushed(&revs, limit + inspected.len() + 1).await?
    } else {
        HashSet::new()
    };
    let refs = repo.refs_by_commit().await?;

    let parents: HashMap<&str, &[String]> = raw
        .iter()
        .map(|c| (c.sha.as_str(), c.parents.as_slice()))
        .collect();
    let mut default_reach = default_tip
        .as_deref()
        .map(|tip| reachable_within(tip, &parents))
        .unwrap_or_default();

    let mut by_commit: HashMap<String, Vec<CommitWorktreeRef>> = HashMap::new();
    for i in &inspected {
        let Some(head) = i.wt.head.as_deref() else {
            continue;
        };
        for sha in reachable_within(head, &parents) {
            by_commit
                .entry(sha.clone())
                .or_default()
                .push(CommitWorktreeRef {
                    worktree_id: i.id.clone(),
                    branch: i.wt.branch.clone(),
                    is_head: sha == head,
                });
        }
    }

    for sha in &outside {
        for i in &inspected {
            let Some(head) = i.wt.head.as_deref() else {
                continue;
            };
            let known = by_commit
                .get(sha)
                .map(|refs| refs.iter().any(|r| r.worktree_id == i.id))
                .unwrap_or(false);
            if !known && repo.is_ancestor(sha, head).await {
                by_commit
                    .entry(sha.clone())
                    .or_default()
                    .push(CommitWorktreeRef {
                        worktree_id: i.id.clone(),
                        branch: i.wt.branch.clone(),
                        is_head: sha == head,
                    });
            }
        }
        if let Some(tip) = default_tip.as_deref() {
            if repo.is_ancestor(sha, tip).await {
                default_reach.insert(sha.clone());
            }
        }
    }

    let commits: Vec<CommitSummary> = raw
        .into_iter()
        .map(|c| {
            let sha = c.sha.clone();
            let mut summary = c.into_summary();
            let mut worktrees = by_commit.remove(&sha).unwrap_or_default();
            worktrees.sort_by(|a, b| (&a.branch, &a.worktree_id).cmp(&(&b.branch, &b.worktree_id)));
            summary.worktrees = worktrees;
            summary.on_default_branch = default_reach.contains(&sha);
            summary.unpushed = has_remote_refs.then(|| unpushed.contains(&sha));
            summary.refs = refs.get(&sha).cloned().unwrap_or_default();
            summary
        })
        .collect();

    let working_trees = inspected
        .iter()
        .filter(|i| !i.changed.is_empty())
        .map(|i| WorkingTreeChange {
            worktree_id: i.id.clone(),
            branch: i.wt.branch.clone(),
            changed_files: i.changed.len() as u32,
            files: i
                .changed
                .iter()
                .take(WORKING_TREE_FILE_PREVIEW)
                .cloned()
                .collect(),
        })
        .collect();

    Ok(CommitTimeline {
        commits,
        working_trees,
        worktrees_total: inspected.len() as u32,
        truncated,
        has_remote_refs,
        default_branch: project.default_branch.clone(),
        generated_at: Utc::now(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::process::Command;

    pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Test Author")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test Author")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    pub(crate) fn commit_file(dir: &Path, name: &str, content: &str, message: &str) -> String {
        std::fs::write(dir.join(name), content).unwrap();
        git(dir, &["add", "--", name]);
        git(dir, &["commit", "-q", "-m", message]);
        git(dir, &["rev-parse", "HEAD"])
    }

    pub(crate) fn init_repo(dir: &Path) {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
    }

    fn project(dir: &Path) -> ProjectRef {
        ProjectRef {
            id: "p1".into(),
            name: "demo".into(),
            repo_path: dir.to_path_buf(),
            default_branch: "main".into(),
        }
    }

    #[tokio::test]
    async fn a_commit_shared_by_several_worktrees_is_one_row_listing_all_of_them() {
        let temp = tempfile::TempDir::new().unwrap();
        let main = temp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        let c1 = commit_file(&main, "a.txt", "1\n", "first");
        let c2 = commit_file(&main, "a.txt", "2\n", "second");

        // Worktree A starts on c2 (shares it with main), worktree B is detached on c1.
        let wt_a = temp.path().join("wt-a");
        let wt_b = temp.path().join("wt-b");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat-a",
                wt_a.to_str().unwrap(),
                &c2,
            ],
        );
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                wt_b.to_str().unwrap(),
                &c1,
            ],
        );

        let timeline = build_timeline(&project(&main), 50).await.unwrap();
        assert_eq!(timeline.worktrees_total, 3);
        let rows_c2: Vec<_> = timeline.commits.iter().filter(|c| c.sha == c2).collect();
        assert_eq!(rows_c2.len(), 1, "one row per SHA");
        let branches: Vec<_> = rows_c2[0]
            .worktrees
            .iter()
            .filter_map(|w| w.branch.clone())
            .collect();
        assert_eq!(branches, vec!["feat-a".to_string(), "main".to_string()]);
        assert!(rows_c2[0].worktrees.iter().all(|w| w.is_head));

        let rows_c1: Vec<_> = timeline.commits.iter().filter(|c| c.sha == c1).collect();
        assert_eq!(rows_c1.len(), 1);
        // All three worktrees contain c1; only the detached one has it as HEAD.
        assert_eq!(rows_c1[0].worktrees.len(), 3);
        assert_eq!(rows_c1[0].worktrees.iter().filter(|w| w.is_head).count(), 1);
        assert!(rows_c1[0].on_default_branch);
        assert_eq!(
            rows_c1[0].unpushed, None,
            "no remote refs: unpushed is meaningless"
        );

        // A commit made in worktree A belongs to A only.
        let d1 = commit_file(&wt_a, "b.txt", "x\n", "agent work");
        let timeline = build_timeline(&project(&main), 50).await.unwrap();
        let row = timeline.commits.iter().find(|c| c.sha == d1).unwrap();
        assert_eq!(row.worktrees.len(), 1);
        assert_eq!(row.worktrees[0].branch.as_deref(), Some("feat-a"));
        assert!(!row.on_default_branch);
        let c2_row = timeline.commits.iter().find(|c| c.sha == c2).unwrap();
        assert_eq!(
            c2_row.worktrees.len(),
            2,
            "main and feat-a, not the detached older worktree"
        );
        assert_eq!(timeline.commits.iter().filter(|c| c.sha == d1).count(), 1);
    }

    #[tokio::test]
    async fn worktrees_report_head_dirty_state_and_commits_ahead_of_the_default_branch() {
        let temp = tempfile::TempDir::new().unwrap();
        let main = temp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        commit_file(&main, "a.txt", "1\n", "first");
        let wt = temp.path().join("wt");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "agent/task-1",
                wt.to_str().unwrap(),
            ],
        );
        let head = commit_file(&wt, "b.txt", "x\n", "agent commit");
        std::fs::write(wt.join("untracked.txt"), "wip\n").unwrap();

        let mut links = TaskLinks::new();
        links.insert(
            normalize_path_key(wt.to_str().unwrap()),
            WorktreeTaskLink {
                task_id: "t1".into(),
                issue_identifier: "SYM-1".into(),
                issue_title: "Do it".into(),
                task_status: symphony_core::TaskStatus::Running,
                provider: symphony_core::ProviderKind::Simulated,
                active_session_id: Some("s1".into()),
            },
        );
        let list = list_worktrees(&project(&main), &links).await.unwrap();
        assert_eq!(list.len(), 2);
        let main_wt = list.iter().find(|w| w.kind == WorktreeKind::Main).unwrap();
        assert!(!main_wt.dirty);
        assert_eq!(main_wt.ahead_of_default, Some(0));
        let agent = list
            .iter()
            .find(|w| w.branch.as_deref() == Some("agent/task-1"))
            .unwrap();
        assert_eq!(agent.kind, WorktreeKind::Symphony);
        assert_eq!(agent.head_sha.as_deref(), Some(head.as_str()));
        assert_eq!(agent.head_subject.as_deref(), Some("agent commit"));
        assert!(agent.dirty && agent.changed_files == 1);
        assert_eq!(agent.ahead_of_default, Some(1));
        assert_eq!(agent.task.as_ref().unwrap().issue_identifier, "SYM-1");

        let timeline = build_timeline(&project(&main), 50).await.unwrap();
        assert_eq!(timeline.working_trees.len(), 1);
        assert_eq!(
            timeline.working_trees[0].files,
            vec!["untracked.txt".to_string()]
        );
    }

    #[tokio::test]
    async fn unpushed_commits_are_those_no_remote_ref_contains() {
        let temp = tempfile::TempDir::new().unwrap();
        let origin = temp.path().join("origin.git");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let main = temp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        git(
            &main,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        let pushed = commit_file(&main, "a.txt", "1\n", "pushed");
        git(&main, &["push", "-q", "-u", "origin", "main"]);
        let local = commit_file(&main, "a.txt", "2\n", "local only");

        let timeline = build_timeline(&project(&main), 50).await.unwrap();
        assert!(timeline.has_remote_refs);
        let find = |sha: &str| timeline.commits.iter().find(|c| c.sha == sha).unwrap();
        assert_eq!(find(&pushed).unpushed, Some(false));
        assert_eq!(find(&local).unpushed, Some(true));
        assert!(find(&pushed).refs.iter().any(|r| r == "origin/main"));
    }

    #[tokio::test]
    async fn a_worktree_older_than_the_window_still_reports_every_worktree_that_contains_its_commit(
    ) {
        let temp = tempfile::TempDir::new().unwrap();
        let main = temp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        let old = commit_file(
            &main, "a.txt", "0
", "old",
        );
        // A detached worktree stays on the old commit while main moves on.
        let wt = temp.path().join("wt-old");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                wt.to_str().unwrap(),
                &old,
            ],
        );
        for n in 1..6 {
            commit_file(
                &main,
                "a.txt",
                &format!(
                    "{n}
"
                ),
                &format!("c{n}"),
            );
        }

        let timeline = build_timeline(&project(&main), 3).await.unwrap();
        assert!(timeline.truncated);
        let row = timeline
            .commits
            .iter()
            .find(|c| c.sha == old)
            .expect("the worktree's HEAD is listed");
        // Main contains the old commit although the commits between are not listed.
        let branches: Vec<_> = row
            .worktrees
            .iter()
            .map(|w| (w.branch.clone(), w.is_head))
            .collect();
        assert_eq!(branches.len(), 2, "{branches:?}");
        assert!(branches.contains(&(Some("main".to_string()), false)));
        assert!(
            branches.contains(&(None, true)),
            "the detached worktree has it as HEAD"
        );
        assert!(
            row.on_default_branch,
            "it is an ancestor of the default branch"
        );
        assert_eq!(timeline.commits.iter().filter(|c| c.sha == old).count(), 1);
    }

    #[tokio::test]
    async fn the_limit_truncates_and_reports_it() {
        let temp = tempfile::TempDir::new().unwrap();
        let main = temp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        for n in 0..5 {
            commit_file(&main, "a.txt", &format!("{n}\n"), &format!("c{n}"));
        }
        let timeline = build_timeline(&project(&main), 3).await.unwrap();
        assert_eq!(timeline.commits.len(), 3);
        assert!(timeline.truncated);
        assert_eq!(timeline.commits[0].subject, "c4");
    }
}
