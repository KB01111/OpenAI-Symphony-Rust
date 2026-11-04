//! Keeping the UI current without it polling the repository.
//!
//! Two sources, because neither is complete on its own:
//!
//! - A filesystem watcher on the repository's git directory. It reacts to what changes history:
//!   a branch moving (`refs/`, `packed-refs`), a worktree's `HEAD`, and `index` (staging and
//!   commits). Object writes, logs and lock files are ignored.
//! - A periodic reconcile of every worktree's HEAD and uncommitted-file count. Edits to files in a
//!   working tree never touch the git directory until they are staged, so only a poll sees them,
//!   and a poll also covers events the watcher missed (network drives, overflowed buffers).
//!
//! Watching starts when a Knowledge Base view asks for it and stops by itself after a while with
//! no requests, so a closed page costs nothing.

use crate::git::GitRepo;
use crate::service::{KnowledgeService, EVENT_REPO_CHANGED};
use crate::timeline::ProjectRef;
use notify::{Event, RecursiveMode, Watcher};
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const DEBOUNCE: Duration = Duration::from_millis(500);
const RECONCILE_EVERY: Duration = Duration::from_secs(30);
/// Watching stops this long after the last request that asked for it.
const IDLE_AFTER: Duration = Duration::from_secs(10 * 60);

/// Shared between the service (which refreshes `last_touch`) and the watching task.
pub struct WatchState {
    pub repo_path: std::path::PathBuf,
    last_touch: Mutex<Instant>,
}

impl WatchState {
    pub fn new(repo_path: std::path::PathBuf) -> Self {
        Self {
            repo_path,
            last_touch: Mutex::new(Instant::now()),
        }
    }

    pub fn touch(&self) {
        *self.last_touch.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle(&self) -> bool {
        self.last_touch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
            > IDLE_AFTER
    }
}

/// An event path under the git directory that means history or staging changed.
pub fn is_relevant(common_dir: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(common_dir) else {
        return false;
    };
    let parts: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let Some(last) = parts.last() else {
        return false;
    };
    if last.ends_with(".lock") {
        return false;
    }
    match parts.first().map(String::as_str) {
        Some("refs") => true,
        Some("HEAD") | Some("packed-refs") | Some("index") if parts.len() == 1 => true,
        Some("worktrees") => parts.len() == 3 && matches!(last.as_str(), "HEAD" | "index"),
        _ => false,
    }
}

/// A cheap description of "what a viewer would see": each worktree's HEAD and which files it has
/// uncommitted (the paths, not their number: swapping one dirty file for another changes the list
/// a viewer sees).
async fn fingerprint(repo: &GitRepo) -> String {
    use std::hash::{Hash, Hasher};
    let Ok(worktrees) = repo.worktrees().await else {
        return String::new();
    };
    let mut parts = Vec::with_capacity(worktrees.len());
    for wt in worktrees.iter().filter(|w| !w.bare) {
        let mut changed = if Path::new(&wt.path).is_dir() {
            GitRepo::status_paths(Path::new(&wt.path)).await
        } else {
            Vec::new()
        };
        changed.sort();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        changed.hash(&mut hasher);
        parts.push(format!(
            "{}|{}|{}|{}:{:x}",
            wt.path,
            wt.head.as_deref().unwrap_or(""),
            wt.branch.as_deref().unwrap_or(""),
            changed.len(),
            hasher.finish()
        ));
    }
    parts.join("\n")
}

/// Watch `project` until it goes idle. Removal from the service's table is done by the caller
/// when this returns.
pub async fn run(service: Arc<KnowledgeService>, project: ProjectRef, state: Arc<WatchState>) {
    let repo = GitRepo::new(&project.repo_path);
    let common = match repo.common_dir().await {
        Ok(dir) => dir,
        Err(error) => {
            tracing::warn!(project = %project.id, %error, "knowledge: cannot locate the git directory to watch");
            return;
        }
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();
    let filter_dir = common.clone();
    let watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if let Ok(event) = result {
            if event.paths.iter().any(|p| is_relevant(&filter_dir, p)) {
                let _ = tx.send(());
            }
        }
    });
    // Keep the watcher alive for the loop; without it the poll below still works.
    let _watcher = match watcher {
        Ok(mut watcher) => match watcher.watch(&common, RecursiveMode::Recursive) {
            Ok(()) => Some(watcher),
            Err(error) => {
                tracing::warn!(project = %project.id, %error, "knowledge: git directory watch failed; polling only");
                None
            }
        },
        Err(error) => {
            tracing::warn!(project = %project.id, %error, "knowledge: filesystem watcher unavailable; polling only");
            None
        }
    };

    let mut last = fingerprint(&repo).await;
    let mut tick = tokio::time::interval(RECONCILE_EVERY);
    tick.tick().await;
    loop {
        tokio::select! {
            changed = rx.recv() => {
                if changed.is_none() {
                    break;
                }
                // A commit touches several files in quick succession: collapse them.
                tokio::time::sleep(DEBOUNCE).await;
                while rx.try_recv().is_ok() {}
                last = fingerprint(&repo).await;
                service.publish_repo_changed(&project.id, "git");
            }
            _ = tick.tick() => {
                if state.idle() {
                    // Decide under the table lock: `watch_project` touches while holding it, so a
                    // request that arrives after this check cannot be left without a watcher.
                    let mut watchers = service.watchers.lock().unwrap_or_else(|e| e.into_inner());
                    if state.idle() {
                        if watchers.get(&project.id).is_some_and(|s| Arc::ptr_eq(s, &state)) {
                            watchers.remove(&project.id);
                        }
                        break;
                    }
                }
                let now = fingerprint(&repo).await;
                if now != last {
                    last = now;
                    service.publish_repo_changed(&project.id, "reconcile");
                }
            }
        }
    }
}

impl KnowledgeService {
    pub(crate) fn publish_repo_changed(&self, project_id: &str, reason: &str) {
        self.publish_event(EVENT_REPO_CHANGED, project_id, json!({ "reason": reason }));
    }

    /// Make sure live updates are flowing for `project` (and keep them going: call this from the
    /// requests that render live data).
    pub fn watch_project(self: &Arc<Self>, project: &ProjectRef) {
        let state = {
            let mut watchers = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = watchers.get(&project.id) {
                if existing.repo_path == project.repo_path {
                    existing.touch();
                    return;
                }
            }
            let state = Arc::new(WatchState::new(project.repo_path.clone()));
            watchers.insert(project.id.clone(), state.clone());
            state
        };
        let service = self.clone();
        let project = project.clone();
        tokio::spawn(async move {
            run(service.clone(), project.clone(), state.clone()).await;
            let mut watchers = service.watchers.lock().unwrap_or_else(|e| e.into_inner());
            // Only remove our own entry: a restart for another path may have replaced it.
            if watchers
                .get(&project.id)
                .map(|s| Arc::ptr_eq(s, &state))
                .unwrap_or(false)
            {
                watchers.remove(&project.id);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_history_and_staging_files_are_relevant() {
        let common = Path::new("/repo/.git");
        let yes = |p: &str| is_relevant(common, Path::new(p));
        assert!(yes("/repo/.git/refs/heads/main"));
        assert!(yes("/repo/.git/HEAD"));
        assert!(yes("/repo/.git/packed-refs"));
        assert!(yes("/repo/.git/index"));
        assert!(yes("/repo/.git/worktrees/wt1/HEAD"));
        assert!(yes("/repo/.git/worktrees/wt1/index"));
        assert!(!yes("/repo/.git/index.lock"));
        assert!(!yes("/repo/.git/refs/heads/main.lock"));
        assert!(!yes("/repo/.git/objects/ab/cdef"));
        assert!(!yes("/repo/.git/logs/HEAD"));
        assert!(!yes("/repo/.git/worktrees/wt1/logs/HEAD"));
        assert!(!yes("/repo/.git/worktrees/wt1/gitdir"));
        assert!(!yes("/elsewhere/refs/heads/main"));
        assert!(!yes("/repo/src/index"));
    }
}
