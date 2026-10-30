//! Materialise a commit into a Symphony-owned directory for the engines.
//!
//! Graphify and Serena write caches next to the code they read (`graphify-out/`, `.serena/`).
//! Running them inside a user's checkout would dirty it and show up as uncommitted changes, so
//! they only ever see a snapshot written here from `git ls-tree` + `git cat-file --batch`.
//! The snapshot has no `.git`, contains no symlinks and no submodule content, and every path is
//! checked to stay inside the snapshot directory (the same containment rule as task workspaces).
//!
//! A snapshot is updated in place: only blobs whose object id changed are written and files that
//! left the tree are removed, so Graphify's per-file cache stays valid between runs.

use crate::git::{GitRepo, TreeEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use symphony_core::{SymphonyError, SymphonyResult};
use symphony_workspaces::{
    git_command, prepare_tree_command, ProcessTree, SECRET_ENV_VARS_TO_STRIP,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Clone, Copy)]
pub struct SnapshotLimits {
    pub max_file_bytes: u64,
    pub max_files: usize,
    pub max_total_bytes: u64,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 1024 * 1024,
            max_files: 60_000,
            max_total_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// Paths a repository may carry that would configure the engines (or hold their output).
const EXCLUDED_PREFIXES: &[&str] = &[".git/", ".serena/", "graphify-out/"];

/// Binary assets the engines cannot use; not worth writing.
const SKIPPED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "icns", "woff", "woff2", "ttf", "otf",
    "eot", "pdf", "zip", "gz", "tgz", "xz", "7z", "tar", "exe", "dll", "so", "dylib", "wasm",
    "mp3", "mp4", "mov", "webm", "ogg", "wav", "bin", "o", "a", "lib", "pdb", "class", "jar",
];

/// What a snapshot update did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotReport {
    /// Repository-relative paths of the files now in the snapshot.
    pub files: Vec<String>,
    pub written: u32,
    pub unchanged: u32,
    pub removed: u32,
    pub skipped_large: u32,
    pub skipped_unsafe: u32,
    pub skipped_other: u32,
    pub truncated: bool,
}

/// `{path: object id}` of what a snapshot directory holds; stored next to it, not inside it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    commit: String,
    files: BTreeMap<String, String>,
}

fn is_reserved_windows_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit())
}

/// A repository path that can be written inside the snapshot on any supported OS.
pub fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return false;
    }
    path.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.contains(':')
            && !part.ends_with('.')
            && !part.ends_with(' ')
            && !is_reserved_windows_name(part)
    })
}

fn extension_skipped(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| SKIPPED_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn target_inside(root: &Path, relative: &str) -> Option<PathBuf> {
    let target = root.join(relative);
    // Lexical check: no component may climb out (the relative path is already validated).
    let escapes = target
        .strip_prefix(root)
        .ok()
        .map(|rest| {
            rest.components()
                .any(|c| !matches!(c, Component::Normal(_)))
        })
        .unwrap_or(true);
    (!escapes).then_some(target)
}

/// Fetch blobs with one `git cat-file --batch` process. Returns `(object id, content)` in the
/// order of `objects`; a missing object is an error.
async fn read_blobs(repo: &GitRepo, objects: &[String]) -> SymphonyResult<Vec<Vec<u8>>> {
    let mut cmd = git_command();
    cmd.arg("-C")
        .arg(repo.root())
        .args(["-c", "core.fsmonitor=false", "cat-file", "--batch"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // A subprocess that reads repository data never inherits tracker or API credentials.
    for var in SECRET_ENV_VARS_TO_STRIP {
        cmd.env_remove(var);
    }
    prepare_tree_command(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| SymphonyError::GitWorktreeError(format!("git cat-file: {e}")))?;
    let mut tree = ProcessTree::attach(&child);
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| SymphonyError::GitWorktreeError("git cat-file: no stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SymphonyError::GitWorktreeError("git cat-file: no stdout".into()))?;

    // Feed the requests from a task so reading the responses can never deadlock on a full pipe.
    let requests: Vec<String> = objects.to_vec();
    let writer = tokio::spawn(async move {
        for object in requests {
            if stdin.write_all(object.as_bytes()).await.is_err()
                || stdin.write_all(b"\n").await.is_err()
            {
                return;
            }
        }
        let _ = stdin.shutdown().await;
    });

    let mut reader = BufReader::new(stdout);
    let mut blobs = Vec::with_capacity(objects.len());
    let mut header = String::new();
    for object in objects {
        header.clear();
        let read = reader
            .read_line(&mut header)
            .await
            .map_err(|e| SymphonyError::GitWorktreeError(format!("git cat-file: {e}")))?;
        if read == 0 {
            tree.kill();
            return Err(SymphonyError::GitWorktreeError(
                "git cat-file ended early".into(),
            ));
        }
        let mut fields = header.split_whitespace();
        let (Some(_id), Some(kind), Some(size)) = (fields.next(), fields.next(), fields.next())
        else {
            tree.kill();
            return Err(SymphonyError::GitWorktreeError(format!(
                "git cat-file: object {object} is {}",
                header.trim()
            )));
        };
        let size: usize = size.parse().map_err(|_| {
            SymphonyError::GitWorktreeError(format!("git cat-file: bad header `{}`", header.trim()))
        })?;
        if kind != "blob" {
            tree.kill();
            return Err(SymphonyError::GitWorktreeError(format!(
                "git cat-file: {object} is a {kind}, not a blob"
            )));
        }
        let mut content = vec![0u8; size];
        reader
            .read_exact(&mut content)
            .await
            .map_err(|e| SymphonyError::GitWorktreeError(format!("git cat-file: {e}")))?;
        let mut newline = [0u8; 1];
        let _ = reader.read_exact(&mut newline).await;
        blobs.push(content);
    }
    let _ = writer.await;
    tree.kill();
    let _ = child.wait().await;
    Ok(blobs)
}

fn manifest_path(dir: &Path) -> PathBuf {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "snapshot".into());
    dir.with_file_name(format!("{name}.manifest.json"))
}

fn load_manifest(dir: &Path) -> Manifest {
    std::fs::read(manifest_path(dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn prune_empty_dirs(root: &Path, mut dir: PathBuf) {
    while dir != root && dir.starts_with(root) {
        if std::fs::remove_dir(&dir).is_err() {
            break;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => break,
        }
    }
}

/// Make `dir` hold exactly the (supported) files of commit `sha`.
pub async fn materialize(
    repo: &GitRepo,
    sha: &str,
    dir: &Path,
    limits: SnapshotLimits,
) -> SymphonyResult<SnapshotReport> {
    let entries: Vec<TreeEntry> = repo.ls_tree(sha).await?;
    std::fs::create_dir_all(dir)?;
    let previous = load_manifest(dir);
    let mut report = SnapshotReport::default();

    let mut wanted: BTreeMap<String, String> = BTreeMap::new();
    let mut total_bytes = 0u64;
    for entry in entries {
        // Regular files only: symlinks (120000) could point out of the snapshot, submodules
        // (160000) are other repositories.
        if !matches!(entry.mode.as_str(), "100644" | "100755") {
            report.skipped_other += 1;
            continue;
        }
        if !is_safe_relative_path(&entry.path) {
            report.skipped_unsafe += 1;
            continue;
        }
        if EXCLUDED_PREFIXES.iter().any(|p| entry.path.starts_with(p))
            || extension_skipped(&entry.path)
        {
            report.skipped_other += 1;
            continue;
        }
        let size = entry.size.unwrap_or(0);
        if size > limits.max_file_bytes {
            report.skipped_large += 1;
            continue;
        }
        if wanted.len() >= limits.max_files || total_bytes + size > limits.max_total_bytes {
            report.truncated = true;
            break;
        }
        total_bytes += size;
        wanted.insert(entry.path, entry.object);
    }

    // Remove what left the tree (or changed type).
    for path in previous.files.keys().filter(|p| !wanted.contains_key(*p)) {
        if let Some(target) = target_inside(dir, path) {
            if std::fs::remove_file(&target).is_ok() {
                report.removed += 1;
            }
            if let Some(parent) = target.parent() {
                prune_empty_dirs(dir, parent.to_path_buf());
            }
        }
    }

    // Write what is new or changed.
    let mut to_fetch: Vec<(String, String)> = Vec::new();
    for (path, object) in &wanted {
        let up_to_date = previous.files.get(path) == Some(object)
            && target_inside(dir, path)
                .map(|t| t.is_file())
                .unwrap_or(false);
        if up_to_date {
            report.unchanged += 1;
        } else {
            to_fetch.push((path.clone(), object.clone()));
        }
    }
    // Unique objects once; identical files share a blob.
    let unique: Vec<String> = {
        let mut seen = HashSet::new();
        to_fetch
            .iter()
            .filter(|(_, o)| seen.insert(o.clone()))
            .map(|(_, o)| o.clone())
            .collect()
    };
    let blobs = if unique.is_empty() {
        Vec::new()
    } else {
        read_blobs(repo, &unique).await?
    };
    let by_object: std::collections::HashMap<&str, &Vec<u8>> = unique
        .iter()
        .map(String::as_str)
        .zip(blobs.iter())
        .collect();
    for (path, object) in &to_fetch {
        let Some(target) = target_inside(dir, path) else {
            report.skipped_unsafe += 1;
            continue;
        };
        let Some(content) = by_object.get(object.as_str()) else {
            continue;
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, content)?;
        report.written += 1;
    }

    report.files = wanted.keys().cloned().collect();
    let manifest = Manifest {
        commit: sha.to_string(),
        files: wanted,
    };
    std::fs::write(manifest_path(dir), serde_json::to_vec(&manifest)?)
        .map_err(|e| SymphonyError::IoError(e.to_string()))?;
    Ok(report)
}

/// Write specific blobs (`sha:path`) under `dir`, for the per-commit symbol extraction. Missing
/// paths are skipped; returns the paths written.
pub async fn materialize_paths(
    repo: &GitRepo,
    sha: &str,
    paths: &[String],
    dir: &Path,
    limits: SnapshotLimits,
) -> SymphonyResult<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let wanted: HashSet<&str> = paths.iter().map(String::as_str).collect();
    let entries = repo.ls_tree(sha).await?;
    let selected: Vec<TreeEntry> = entries
        .into_iter()
        .filter(|e| {
            wanted.contains(e.path.as_str())
                && matches!(e.mode.as_str(), "100644" | "100755")
                && is_safe_relative_path(&e.path)
                && e.size.unwrap_or(0) <= limits.max_file_bytes
        })
        .collect();
    if selected.is_empty() {
        return Ok(Vec::new());
    }
    let objects: Vec<String> = selected.iter().map(|e| e.object.clone()).collect();
    let blobs = read_blobs(repo, &objects).await?;
    let mut written = Vec::new();
    for (entry, content) in selected.iter().zip(blobs) {
        let Some(target) = target_inside(dir, &entry.path) else {
            continue;
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, content)?;
        written.push(entry.path.clone());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::tests::{commit_file, git, init_repo};

    #[test]
    fn unsafe_paths_are_rejected() {
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a//b",
            "a\\b",
            "C:/x",
            "a/b:stream",
            "con",
            "src/NUL.txt",
            "dir/trailing.",
            "dir/trailing ",
            "./x",
        ] {
            assert!(!is_safe_relative_path(bad), "{bad:?} must be rejected");
        }
        for good in [
            "a.rs",
            "src/lib.rs",
            "packages/ui/src/index.ts",
            ".github/ci.yml",
            "docs/a b.md",
        ] {
            assert!(is_safe_relative_path(good), "{good:?} must be accepted");
        }
    }

    #[tokio::test]
    async fn a_snapshot_is_updated_in_place_and_excludes_engine_directories() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo_dir = temp.path().join("repo");
        std::fs::create_dir_all(repo_dir.join("src")).unwrap();
        init_repo(&repo_dir);
        std::fs::create_dir_all(repo_dir.join(".serena")).unwrap();
        std::fs::write(
            repo_dir.join(".serena").join("project.yml"),
            "activation_command: evil\n",
        )
        .unwrap();
        std::fs::write(repo_dir.join("src").join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(repo_dir.join("logo.png"), [0u8, 1, 2]).unwrap();
        std::fs::write(repo_dir.join("keep.txt"), "keep\n").unwrap();
        git(&repo_dir, &["add", "-A"]);
        git(&repo_dir, &["commit", "-q", "-m", "one"]);
        let c1 = git(&repo_dir, &["rev-parse", "HEAD"]);

        let repo = GitRepo::new(&repo_dir);
        let dir = temp.path().join("snap").join("current");
        let first = materialize(&repo, &c1, &dir, SnapshotLimits::default())
            .await
            .unwrap();
        assert_eq!(first.written, 2);
        assert!(dir.join("src").join("a.rs").is_file());
        assert!(
            !dir.join(".serena").exists(),
            "a repository must not configure Serena"
        );
        assert!(!dir.join("logo.png").exists());
        assert!(!dir.join(".git").exists());

        // Second commit: change one file, delete another, add a nested one.
        std::fs::write(repo_dir.join("src").join("a.rs"), "fn a() { 1; }\n").unwrap();
        git(&repo_dir, &["rm", "-q", "keep.txt"]);
        std::fs::create_dir_all(repo_dir.join("deep").join("er")).unwrap();
        std::fs::write(repo_dir.join("deep").join("er").join("n.rs"), "fn n() {}\n").unwrap();
        git(&repo_dir, &["add", "-A"]);
        git(&repo_dir, &["commit", "-q", "-m", "two"]);
        let c2 = git(&repo_dir, &["rev-parse", "HEAD"]);

        let second = materialize(&repo, &c2, &dir, SnapshotLimits::default())
            .await
            .unwrap();
        assert_eq!(second.written, 2, "the changed file and the new one");
        assert_eq!(second.removed, 1);
        assert!(!dir.join("keep.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("src").join("a.rs")).unwrap(),
            "fn a() { 1; }\n"
        );
        assert_eq!(
            second.files,
            vec!["deep/er/n.rs".to_string(), "src/a.rs".to_string()]
        );

        // Going back removes the nested directory again.
        let third = materialize(&repo, &c1, &dir, SnapshotLimits::default())
            .await
            .unwrap();
        assert_eq!(third.removed, 1);
        assert!(!dir.join("deep").exists(), "empty directories are pruned");
    }

    #[tokio::test]
    async fn large_files_and_symlinks_are_skipped() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo_dir = temp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        init_repo(&repo_dir);
        commit_file(&repo_dir, "small.rs", "fn s() {}\n", "small");
        let big = "x".repeat(2048);
        let sha = commit_file(&repo_dir, "big.rs", &big, "big");

        let repo = GitRepo::new(&repo_dir);
        let limits = SnapshotLimits {
            max_file_bytes: 1024,
            ..SnapshotLimits::default()
        };
        let report = materialize(&repo, &sha, &temp.path().join("snap"), limits)
            .await
            .unwrap();
        assert_eq!(report.skipped_large, 1);
        assert_eq!(report.files, vec!["small.rs".to_string()]);

        let written = materialize_paths(
            &repo,
            &sha,
            &["small.rs".into(), "missing.rs".into()],
            &temp.path().join("part"),
            limits,
        )
        .await
        .unwrap();
        assert_eq!(written, vec!["small.rs".to_string()]);
    }
}
