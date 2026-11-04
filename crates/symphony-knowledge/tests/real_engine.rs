//! End-to-end run against the real Graphify. Skipped unless `SYMPHONY_KB_REAL_ENGINES=1`; point
//! `SYMPHONY_GRAPHIFY` at the executable when it is not on `PATH`.
//!
//! It indexes a small generated repository, then checks that a commit's changed symbols are
//! matched to the index by id and that the symbols that call them come back as affected.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use symphony_knowledge::engine::EngineConfig;
use symphony_knowledge::model::{IndexStatus, NodeKind, SymbolChange};
use symphony_knowledge::service::{GraphQuery, KnowledgeService};
use symphony_knowledge::timeline::ProjectRef;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, path: &str, content: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, content).unwrap();
}

/// A two-crate repository whose last commit changes `helper()` only; returns its HEAD.
fn make_repo(temp: &tempfile::TempDir) -> (std::path::PathBuf, String) {
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    write(
        &repo,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/engine_core\", \"crates/app\"]\nresolver = \"2\"\n",
    );
    write(
        &repo,
        "crates/engine_core/Cargo.toml",
        "[package]\nname = \"engine_core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        &repo,
        "crates/engine_core/src/lib.rs",
        "pub fn compute() -> u32 {\n    helper() + 1\n}\n\nfn helper() -> u32 {\n    1\n}\n\npub fn unrelated() -> u32 {\n    7\n}\n",
    );
    write(&repo, "crates/app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nengine_core = { path = \"../engine_core\" }\n");
    write(
        &repo,
        "crates/app/src/main.rs",
        "use engine_core::compute;\n\nfn run() -> u32 {\n    compute()\n}\n\nfn main() {\n    println!(\"{}\", run());\n}\n",
    );
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    // The commit under inspection changes helper() only.
    write(
        &repo,
        "crates/engine_core/src/lib.rs",
        "pub fn compute() -> u32 {\n    helper() + 1\n}\n\nfn helper() -> u32 {\n    let base = 40;\n    base + 2\n}\n\npub fn unrelated() -> u32 {\n    7\n}\n",
    );
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "change helper"]);
    let change = git(&repo, &["rev-parse", "HEAD"]);
    (repo, change)
}

async fn index_and_wait(
    service: &std::sync::Arc<KnowledgeService>,
    project: &ProjectRef,
) -> symphony_knowledge::model::IndexRunSummary {
    let run = service.start_index(project, None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let current = service.store().run(run.id).await.unwrap().unwrap();
        if current.status != IndexStatus::Running {
            return current;
        }
        assert!(Instant::now() < deadline, "indexing did not finish");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test]
async fn index_and_commit_impact_with_the_real_graphify() {
    if std::env::var("SYMPHONY_KB_REAL_ENGINES").ok().as_deref() != Some("1") {
        eprintln!("skipped: set SYMPHONY_KB_REAL_ENGINES=1 to run against the real Graphify");
        return;
    }
    let temp = tempfile::TempDir::new().unwrap();
    let (repo, change) = make_repo(&temp);

    let data = temp.path().join("data");
    let service = KnowledgeService::open(&data, EngineConfig::from_env())
        .await
        .unwrap();
    let engines = service.engines().await;
    assert!(
        engines.iter().any(|e| e.id == "graphify" && e.available),
        "Graphify must be installed (SYMPHONY_GRAPHIFY): {engines:?}"
    );
    let project = ProjectRef {
        id: "p1".into(),
        name: "demo".into(),
        repo_path: repo.clone(),
        default_branch: "main".into(),
    };

    let finished = index_and_wait(&service, &project).await;
    assert_eq!(
        finished.status,
        IndexStatus::Completed,
        "index failed: {:?}",
        finished.error
    );
    assert_eq!(finished.commit_sha, change);
    assert!(finished.languages.iter().any(|l| l.language == "Rust"));
    // The snapshot is Symphony's, never the repository: nothing was written next to the code.
    assert!(!repo.join("graphify-out").exists() && !repo.join(".serena").exists());
    assert!(
        git(&repo, &["status", "--porcelain"]).is_empty(),
        "the repository must stay clean"
    );

    let hits = service.search(&project, "helper", 10).await.unwrap();
    let helper = hits
        .iter()
        .find(|h| h.node.kind == NodeKind::Function && h.node.label.contains("helper"))
        .unwrap_or_else(|| panic!("helper() not found in {hits:?}"));
    assert_eq!(
        helper.node.file.as_deref(),
        Some("crates/engine_core/src/lib.rs")
    );
    let detail = service.node(&project, &helper.node.id).await.unwrap();
    assert!(
        detail
            .references
            .iter()
            .any(|r| r.node.label.contains("compute")),
        "compute() calls helper(): {:?}",
        detail
            .references
            .iter()
            .map(|r| &r.node.label)
            .collect::<Vec<_>>()
    );
    assert!(
        !detail.recent_commits.is_empty(),
        "history of the file comes from git"
    );

    let modules = service
        .graph(
            &project,
            &GraphQuery {
                level: symphony_knowledge::model::GraphLevel::Modules,
                module: None,
                focus: None,
                depth: 1,
                include_inferred: true,
                include_external: false,
            },
        )
        .await
        .unwrap();
    assert!(modules.nodes.iter().any(|n| n.id == "crates/engine_core"));

    let impact = service
        .commit_impact(&project, &change, Some(3), true)
        .await
        .unwrap();
    assert!(
        impact.unavailable_reason.is_none(),
        "{:?}",
        impact.unavailable_reason
    );
    let lib = impact
        .files
        .iter()
        .find(|f| f.path.ends_with("engine_core/src/lib.rs"))
        .unwrap();
    let changed: Vec<_> = lib
        .symbols
        .iter()
        .map(|s| (s.label.as_str(), s.change, s.in_index))
        .collect();
    assert!(
        changed
            .iter()
            .any(|(label, change, in_index)| label.contains("helper")
                && *change == SymbolChange::Modified
                && *in_index),
        "helper() should be modified and matched to the index: {changed:?}"
    );
    assert!(
        !changed
            .iter()
            .any(|(label, _, _)| label.contains("unrelated")),
        "unrelated() was not touched: {changed:?}"
    );
    let affected: Vec<_> = impact
        .affected
        .iter()
        .map(|a| a.node.label.as_str())
        .collect();
    assert!(
        affected.iter().any(|l| l.contains("compute")),
        "compute() depends on helper(): {affected:?}"
    );
    // A second request is served from the cache with the same content.
    let again = service
        .commit_impact(&project, &change, Some(3), true)
        .await
        .unwrap();
    assert_eq!(again.affected.len(), impact.affected.len());
}

#[tokio::test]
async fn precise_references_with_the_real_serena() {
    if std::env::var("SYMPHONY_KB_REAL_SERENA").ok().as_deref() != Some("1") {
        eprintln!("skipped: set SYMPHONY_KB_REAL_SERENA=1 (and SYMPHONY_KB_REAL_ENGINES=1 settings) to run against the real Serena");
        return;
    }
    let temp = tempfile::TempDir::new().unwrap();
    let (repo, _) = make_repo(&temp);
    let service = KnowledgeService::open(&temp.path().join("data"), EngineConfig::from_env())
        .await
        .unwrap();
    let project = ProjectRef {
        id: "p1".into(),
        name: "demo".into(),
        repo_path: repo.clone(),
        default_branch: "main".into(),
    };
    let finished = index_and_wait(&service, &project).await;
    assert_eq!(
        finished.status,
        IndexStatus::Completed,
        "index failed: {:?}",
        finished.error
    );

    let hits = service.search(&project, "helper", 10).await.unwrap();
    let helper = hits
        .iter()
        .find(|h| h.node.kind == NodeKind::Function && h.node.label.contains("helper"))
        .unwrap();
    let answer = service
        .node_references(&project, &helper.node.id)
        .await
        .unwrap();
    assert!(answer.available, "{:?}", answer.reason);
    assert!(
        answer
            .references
            .iter()
            .any(|r| r.name_path.contains("compute") && r.file == "crates/engine_core/src/lib.rs"),
        "compute() uses helper(): {:?}",
        answer.references
    );
    assert!(answer.definition_start_line.unwrap() <= helper.node.line.unwrap());
    // Serena kept to its own copy: nothing appeared in the repository or the Graphify snapshot.
    assert!(!repo.join(".serena").exists());
    assert!(!service.paths().snapshot_dir("p1").join(".serena").exists());
    assert!(service.paths().serena_dir("p1").join(".serena").exists());
    // A function that nothing calls has no references, and a non-symbol is refused politely.
    let file_node = service
        .search(&project, "lib.rs", 10)
        .await
        .unwrap()
        .into_iter()
        .find(|h| h.node.kind == NodeKind::File)
        .unwrap();
    assert!(
        !service
            .node_references(&project, &file_node.node.id)
            .await
            .unwrap()
            .available
    );
}
