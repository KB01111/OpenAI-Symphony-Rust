//! From a commit to the code it touched: changed line ranges (git) are mapped onto symbols, and
//! the symbols onto the indexed graph to find what depends on them.
//!
//! The graph is built for one commit (the indexed ref), but a commit being inspected may be many
//! commits older or on another branch, so its line numbers cannot be looked up there. Instead the
//! symbols of the *changed files only* are extracted at the commit and at its parent (a few
//! seconds with Graphify), the hunks are mapped onto those, and the resulting symbols are matched
//! to the index by node id (ids are derived from file path and name, so they are stable across
//! commits as long as the file and the symbol keep their names).
//!
//! Graphify reports where a symbol starts but not where it ends, so a symbol is taken to extend
//! to the next one in the file ([`Precision::StartToNext`]); lines between two symbols are
//! attributed to the earlier one and the UI says so.

use crate::engine::{import_graph, is_source_file, read_graph_file, GraphifyEngine};
use crate::git::GitRepo;
use crate::graph::LoadedGraph;
use crate::model::{
    ChangedFile, ChangedSymbol, CommitImpact, DiffHunk, FileChangeStatus, ImpactFile,
    IndexRunSummary, NodeKind, Precision, SymbolChange,
};
use crate::snapshot::{materialize_paths, SnapshotLimits};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use symphony_core::SymphonyResult;
use tokio::sync::Semaphore;

/// Changed source files examined symbol by symbol per commit.
pub const MAX_SYMBOL_FILES: usize = 80;
pub const MAX_AFFECTED: usize = 300;
pub const DEFAULT_IMPACT_DEPTH: u32 = 2;
pub const MAX_IMPACT_DEPTH: u32 = 4;

/// A symbol of one file at one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSymbol {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    pub line: u32,
}

/// Something that can list the symbols of files as they are in a commit.
#[async_trait]
pub trait SymbolExtractor: Send + Sync {
    async fn symbols_at(
        &self,
        repo: &GitRepo,
        sha: &str,
        paths: &[String],
    ) -> SymphonyResult<HashMap<String, Vec<FileSymbol>>>;
}

/// [`SymbolExtractor`] backed by Graphify on a small directory holding just the files.
pub struct GraphifySymbolExtractor {
    engine: GraphifyEngine,
    work_root: PathBuf,
    slots: Arc<Semaphore>,
}

static WORK_COUNTER: AtomicU64 = AtomicU64::new(0);

impl GraphifySymbolExtractor {
    pub fn new(engine: GraphifyEngine, work_root: PathBuf, slots: Arc<Semaphore>) -> Self {
        Self {
            engine,
            work_root,
            slots,
        }
    }
}

#[async_trait]
impl SymbolExtractor for GraphifySymbolExtractor {
    async fn symbols_at(
        &self,
        repo: &GitRepo,
        sha: &str,
        paths: &[String],
    ) -> SymphonyResult<HashMap<String, Vec<FileSymbol>>> {
        if paths.is_empty() {
            return Ok(HashMap::new());
        }
        let _permit = self
            .slots
            .acquire()
            .await
            .map_err(|e| symphony_core::SymphonyError::Conflict(e.to_string()))?;
        let unique = format!(
            "{}-{}-{}",
            std::process::id(),
            WORK_COUNTER.fetch_add(1, Ordering::Relaxed),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let dir = self.work_root.join(unique);
        let input = dir.join("src");
        let output = dir.join("out");
        let result = async {
            let written =
                materialize_paths(repo, sha, paths, &input, SnapshotLimits::default()).await?;
            if written.is_empty() {
                return Ok(HashMap::new());
            }
            let graph_path = self
                .engine
                .extract(&input, &output, Duration::from_secs(180))
                .await?;
            let known: HashSet<String> = written.into_iter().collect();
            let imported = import_graph(&read_graph_file(&graph_path)?, &known)?;
            let mut by_file: HashMap<String, Vec<FileSymbol>> = HashMap::new();
            for node in imported.nodes {
                let (Some(file), Some(line)) = (node.file, node.line) else {
                    continue;
                };
                if node.kind.is_symbol() && known.contains(&file) {
                    by_file.entry(file).or_default().push(FileSymbol {
                        id: node.id,
                        label: node.label,
                        kind: node.kind,
                        line,
                    });
                }
            }
            Ok(by_file)
        }
        .await;
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
}

// ---------------------------------------------------------------------------------------------
// Mapping hunks onto symbols (pure)
// ---------------------------------------------------------------------------------------------

/// A symbol and the lines it is taken to cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extent {
    pub symbol: FileSymbol,
    pub first: u32,
    pub last: u32,
}

/// Symbols with their extents: from the start line to the line before the next symbol that starts
/// later (the last one runs to the end of the file).
pub fn extents(symbols: &[FileSymbol]) -> Vec<Extent> {
    let mut sorted: Vec<&FileSymbol> = symbols.iter().collect();
    sorted.sort_by(|a, b| (a.line, &a.id).cmp(&(b.line, &b.id)));
    let lines: Vec<u32> = sorted.iter().map(|s| s.line).collect();
    sorted
        .iter()
        .map(|symbol| {
            // `lines` is sorted: the first line after this symbol's is a binary search away.
            let next = lines
                .get(lines.partition_point(|&l| l <= symbol.line))
                .copied();
            Extent {
                symbol: (*symbol).clone(),
                first: symbol.line,
                last: next.map(|l| l - 1).unwrap_or(u32::MAX),
            }
        })
        .collect()
}

/// The lines of a hunk on the new side as an inclusive range; a pure deletion is the line it
/// happened after.
fn new_range(hunk: &DiffHunk) -> (u32, u32) {
    if hunk.new_lines == 0 {
        let at = hunk.new_start.max(1);
        (at, at)
    } else {
        (hunk.new_start, hunk.new_start + hunk.new_lines - 1)
    }
}

/// A symbol with the change classification of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedSymbol {
    pub symbol: FileSymbol,
    pub change: SymbolChange,
}

/// The symbols of `file` that the commit added, modified or removed, and the number of hunks
/// that fall outside every symbol. `pre` and `post` are the file's symbols at the parent and at
/// the commit.
pub fn map_file(
    file: &ChangedFile,
    pre: &[FileSymbol],
    post: &[FileSymbol],
) -> (Vec<MappedSymbol>, u32) {
    match file.status {
        FileChangeStatus::Added => {
            let mapped = post
                .iter()
                .cloned()
                .map(|symbol| MappedSymbol {
                    symbol,
                    change: SymbolChange::Added,
                })
                .collect();
            return (mapped, 0);
        }
        FileChangeStatus::Deleted => {
            let mapped = pre
                .iter()
                .cloned()
                .map(|symbol| MappedSymbol {
                    symbol,
                    change: SymbolChange::Removed,
                })
                .collect();
            return (mapped, 0);
        }
        _ => {}
    }

    let key = |s: &FileSymbol| (s.label.clone(), s.kind);
    let mut pre_counts: HashMap<(String, NodeKind), u32> = HashMap::new();
    for symbol in pre {
        *pre_counts.entry(key(symbol)).or_default() += 1;
    }
    let post_extents = extents(post);
    let mut hit: Vec<bool> = vec![false; post_extents.len()];
    let mut unattributed = 0u32;
    for hunk in &file.hunks {
        let (first, last) = new_range(hunk);
        let mut any = false;
        for (i, extent) in post_extents.iter().enumerate() {
            if extent.first <= last && first <= extent.last {
                hit[i] = true;
                any = true;
            }
        }
        if !any {
            unattributed += 1;
        }
    }

    let mut mapped = Vec::new();
    let mut post_counts: HashMap<(String, NodeKind), u32> = HashMap::new();
    for symbol in post {
        *post_counts.entry(key(symbol)).or_default() += 1;
    }
    for (extent, hit) in post_extents.iter().zip(&hit) {
        if !hit {
            continue;
        }
        let k = key(&extent.symbol);
        let change = match pre_counts.get_mut(&k) {
            Some(n) if *n > 0 => {
                *n -= 1;
                SymbolChange::Modified
            }
            _ => SymbolChange::Added,
        };
        mapped.push(MappedSymbol {
            symbol: extent.symbol.clone(),
            change,
        });
    }
    // Symbols that were in the parent and are gone: removed (counted against the post set).
    let mut remaining = post_counts;
    for symbol in pre {
        let k = key(symbol);
        match remaining.get_mut(&k) {
            Some(n) if *n > 0 => *n -= 1,
            _ => mapped.push(MappedSymbol {
                symbol: symbol.clone(),
                change: SymbolChange::Removed,
            }),
        }
    }
    (mapped, unattributed)
}

// ---------------------------------------------------------------------------------------------
// Putting it together
// ---------------------------------------------------------------------------------------------

/// The index an impact is computed against.
pub struct IndexContext<'a> {
    pub graph: &'a LoadedGraph,
    pub run: &'a IndexRunSummary,
}

pub struct ImpactRequest<'a> {
    pub sha: &'a str,
    pub parent: Option<&'a str>,
    pub files: &'a [ChangedFile],
    pub depth: u32,
    pub include_inferred: bool,
    /// The commit's hunk data was cut short, so files late in it have none.
    pub patch_truncated: bool,
}

/// Why the symbol-level part is missing, for the UI.
pub enum SymbolSupport<'a> {
    Available(&'a dyn SymbolExtractor),
    Unavailable(String),
}

pub async fn build_impact(
    repo: &GitRepo,
    request: ImpactRequest<'_>,
    index: Option<IndexContext<'_>>,
    support: SymbolSupport<'_>,
) -> SymphonyResult<CommitImpact> {
    let depth = request.depth.clamp(1, MAX_IMPACT_DEPTH);
    // Added and deleted files are mapped without hunks; a modified file with none, in a patch
    // that was cut short, cannot be attributed to symbols and is counted as not analysed.
    let hunks_missing = |f: &ChangedFile| {
        request.patch_truncated
            && !f.binary
            && f.hunks.is_empty()
            && !matches!(
                f.status,
                FileChangeStatus::Added | FileChangeStatus::Deleted
            )
    };
    let source_files: Vec<&ChangedFile> = request
        .files
        .iter()
        .filter(|f| {
            is_source_file(&f.path) || f.old_path.as_deref().map(is_source_file).unwrap_or(false)
        })
        .collect();
    let (cut_short, source_files): (Vec<&ChangedFile>, Vec<&ChangedFile>) =
        source_files.into_iter().partition(|f| hunks_missing(f));
    let analysed: Vec<&ChangedFile> = source_files
        .iter()
        .copied()
        .take(MAX_SYMBOL_FILES)
        .collect();
    let symbol_files_skipped = (source_files.len() - analysed.len() + cut_short.len()) as u32;

    let mut unavailable_reason = None;
    let mut pre: HashMap<String, Vec<FileSymbol>> = HashMap::new();
    let mut post: HashMap<String, Vec<FileSymbol>> = HashMap::new();
    match &support {
        SymbolSupport::Unavailable(reason) => unavailable_reason = Some(reason.clone()),
        SymbolSupport::Available(extractor) => {
            let post_paths: Vec<String> = analysed
                .iter()
                .filter(|f| f.status != FileChangeStatus::Deleted)
                .map(|f| f.path.clone())
                .collect();
            let pre_paths: Vec<String> = analysed
                .iter()
                .filter(|f| f.status != FileChangeStatus::Added)
                .map(|f| f.old_path.clone().unwrap_or_else(|| f.path.clone()))
                .collect();
            let (after, before) = tokio::join!(
                extractor.symbols_at(repo, request.sha, &post_paths),
                async {
                    match request.parent {
                        Some(parent) => extractor.symbols_at(repo, parent, &pre_paths).await,
                        None => Ok(HashMap::new()),
                    }
                }
            );
            // A file Graphify cannot read, or a timeout, must not take the file list with it:
            // fall back to the file-level answer, the same as when no engine is installed.
            match (after, before) {
                (Ok(after), Ok(before)) => {
                    post = after;
                    pre = before;
                }
                (Err(error), _) | (_, Err(error)) => {
                    unavailable_reason = Some(format!(
                        "Symbols could not be read for this commit ({error}), so changed code is listed by file only."
                    ));
                }
            }
        }
    }

    let mut files: Vec<ImpactFile> = Vec::with_capacity(request.files.len());
    let mut seeds: Vec<String> = Vec::new();
    for file in request.files {
        let analysed_here = analysed.iter().any(|a| a.path == file.path);
        let mut symbols = Vec::new();
        let mut unattributed = 0;
        if analysed_here && unavailable_reason.is_none() {
            let old_path = file.old_path.clone().unwrap_or_else(|| file.path.clone());
            let empty = Vec::new();
            let pre_symbols = pre.get(&old_path).unwrap_or(&empty);
            let post_symbols = post.get(&file.path).unwrap_or(&empty);
            let (mapped, loose) = map_file(file, pre_symbols, post_symbols);
            unattributed = loose;
            for m in mapped {
                let file_path = if m.change == SymbolChange::Removed {
                    old_path.clone()
                } else {
                    file.path.clone()
                };
                let found = index.as_ref().and_then(|ctx| {
                    ctx.graph.node(&m.symbol.id).or_else(|| {
                        ctx.graph
                            .find_symbol(&file_path, &m.symbol.label, m.symbol.kind)
                    })
                });
                if let Some(node) = found {
                    seeds.push(node.id.clone());
                }
                symbols.push(ChangedSymbol {
                    node_id: found.map(|n| n.id.clone()),
                    label: m.symbol.label,
                    kind: m.symbol.kind,
                    file: file_path,
                    line: m.symbol.line,
                    change: m.change,
                    precision: Precision::StartToNext,
                    in_index: found.is_some(),
                });
            }
        }
        symbols.sort_by(|a, b| (a.line, &a.label).cmp(&(b.line, &b.label)));
        let file_node_id = index
            .as_ref()
            .and_then(|ctx| {
                ctx.graph.file_node(&file.path).or_else(|| {
                    file.old_path
                        .as_deref()
                        .and_then(|p| ctx.graph.file_node(p))
                })
            })
            .map(|n| n.id.clone());
        // A file with no attributable symbol (config, docs, constants, a deleted symbol-less
        // file) is a change to the file as a whole.
        if symbols.iter().all(|s| !s.in_index) {
            if let Some(id) = &file_node_id {
                seeds.push(id.clone());
            }
        }
        files.push(ImpactFile {
            path: file.path.clone(),
            status: file.status,
            file_node_id,
            symbols,
            unattributed_hunks: unattributed,
        });
    }

    let (affected, affected_truncated) = match &index {
        Some(ctx) if !seeds.is_empty() => {
            seeds.sort();
            seeds.dedup();
            ctx.graph
                .affected(&seeds, depth, MAX_AFFECTED, request.include_inferred)
        }
        _ => (Vec::new(), false),
    };
    if index.is_none() && unavailable_reason.is_none() {
        unavailable_reason = Some(
            "The repository has not been indexed yet, so the dependents of the changed code are unknown."
                .into(),
        );
    }

    Ok(CommitImpact {
        sha: request.sha.to_string(),
        run_id: index.as_ref().map(|c| c.run.id),
        indexed_sha: index.as_ref().map(|c| c.run.commit_sha.clone()),
        unavailable_reason,
        files,
        affected,
        depth,
        affected_truncated,
        symbol_files_skipped,
        computed_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Confidence, GraphEdge, GraphNode};

    fn sym(id: &str, label: &str, line: u32) -> FileSymbol {
        FileSymbol {
            id: id.into(),
            label: label.into(),
            kind: NodeKind::Function,
            line,
        }
    }

    fn file(status: FileChangeStatus, hunks: Vec<DiffHunk>) -> ChangedFile {
        ChangedFile {
            path: "src/a.rs".into(),
            old_path: None,
            status,
            additions: None,
            deletions: None,
            binary: false,
            hunks,
        }
    }

    fn hunk(new_start: u32, new_lines: u32) -> DiffHunk {
        DiffHunk {
            old_start: new_start,
            old_lines: new_lines,
            new_start,
            new_lines,
        }
    }

    #[test]
    fn a_symbol_extends_to_the_line_before_the_next_symbol() {
        let ext = extents(&[sym("c", "c", 30), sym("a", "a", 5), sym("b", "b", 12)]);
        let ranges: Vec<_> = ext
            .iter()
            .map(|e| (e.symbol.id.as_str(), e.first, e.last))
            .collect();
        assert_eq!(
            ranges,
            vec![("a", 5, 11), ("b", 12, 29), ("c", 30, u32::MAX)]
        );
    }

    #[test]
    fn hunks_map_onto_the_symbols_they_overlap() {
        let symbols = vec![sym("a", "a", 5), sym("b", "b", 12), sym("c", "c", 30)];
        // Lines 14-15 are inside b; line 3 is before every symbol; the deletion after line 29 is in b.
        let f = file(
            FileChangeStatus::Modified,
            vec![
                hunk(14, 2),
                hunk(3, 1),
                DiffHunk {
                    old_start: 29,
                    old_lines: 4,
                    new_start: 29,
                    new_lines: 0,
                },
            ],
        );
        let (mapped, loose) = map_file(&f, &symbols, &symbols);
        let ids: Vec<_> = mapped
            .iter()
            .map(|m| (m.symbol.id.as_str(), m.change))
            .collect();
        assert_eq!(ids, vec![("b", SymbolChange::Modified)]);
        assert_eq!(loose, 1, "line 3 belongs to no symbol");
    }

    #[test]
    fn a_hunk_spanning_two_symbols_marks_both() {
        let symbols = vec![sym("a", "a", 5), sym("b", "b", 12)];
        let f = file(FileChangeStatus::Modified, vec![hunk(10, 4)]);
        let (mapped, _) = map_file(&f, &symbols, &symbols);
        assert_eq!(mapped.len(), 2);
    }

    #[test]
    fn added_and_removed_symbols_are_told_apart_by_name_and_kind() {
        let pre = vec![sym("a", "a", 5), sym("gone", "gone", 20)];
        let post = vec![sym("a", "a", 5), sym("fresh", "fresh", 20)];
        let f = file(FileChangeStatus::Modified, vec![hunk(20, 3)]);
        let (mapped, _) = map_file(&f, &pre, &post);
        let by: HashMap<_, _> = mapped
            .iter()
            .map(|m| (m.symbol.label.as_str(), m.change))
            .collect();
        assert_eq!(by["fresh"], SymbolChange::Added);
        assert_eq!(by["gone"], SymbolChange::Removed);
        assert!(!by.contains_key("a"), "an untouched symbol is not reported");
    }

    #[test]
    fn whole_file_additions_and_deletions_report_every_symbol() {
        let symbols = vec![sym("a", "a", 1), sym("b", "b", 9)];
        let added = map_file(
            &file(FileChangeStatus::Added, vec![hunk(1, 20)]),
            &[],
            &symbols,
        )
        .0;
        assert!(added.iter().all(|m| m.change == SymbolChange::Added) && added.len() == 2);
        let deleted = map_file(&file(FileChangeStatus::Deleted, vec![]), &symbols, &[]).0;
        assert!(deleted.iter().all(|m| m.change == SymbolChange::Removed) && deleted.len() == 2);
    }

    struct FakeExtractor {
        pre: HashMap<String, Vec<FileSymbol>>,
        post: HashMap<String, Vec<FileSymbol>>,
        parent: String,
    }

    #[async_trait]
    impl SymbolExtractor for FakeExtractor {
        async fn symbols_at(
            &self,
            _repo: &GitRepo,
            sha: &str,
            _paths: &[String],
        ) -> SymphonyResult<HashMap<String, Vec<FileSymbol>>> {
            Ok(if sha == self.parent {
                self.pre.clone()
            } else {
                self.post.clone()
            })
        }
    }

    fn node(id: &str, kind: NodeKind, file: &str, line: u32) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: id.into(),
            kind,
            file: Some(file.into()),
            line: Some(line),
            module: Some("m".into()),
            degree: 0,
            members: 1,
        }
    }

    #[tokio::test]
    async fn impact_matches_changed_symbols_to_the_index_and_finds_their_dependents() {
        // Index: caller (other.rs) calls target (src/a.rs).
        let graph = LoadedGraph::new(
            3,
            vec![
                node("file_a", NodeKind::File, "src/a.rs", 1),
                node("target", NodeKind::Function, "src/a.rs", 10),
                node("caller", NodeKind::Function, "src/other.rs", 4),
            ],
            vec![GraphEdge {
                source: "caller".into(),
                target: "target".into(),
                relation: "calls".into(),
                confidence: Confidence::Extracted,
                line: Some(5),
                weight: 1,
            }],
        );
        let run = IndexRunSummary {
            id: 3,
            project_id: "p".into(),
            ref_name: "main".into(),
            commit_sha: "a".repeat(40),
            engine: "graphify".into(),
            engine_version: "0.9.97".into(),
            status: crate::model::IndexStatus::Completed,
            node_count: 3,
            edge_count: 1,
            file_count: 2,
            languages: vec![],
            error: None,
            started_at: Utc::now(),
            finished_at: None,
            duration_ms: None,
        };
        let parent = "b".repeat(40);
        let sha = "c".repeat(40);
        let extractor = FakeExtractor {
            parent: parent.clone(),
            pre: HashMap::from([("src/a.rs".to_string(), vec![sym("target", "target", 10)])]),
            post: HashMap::from([(
                "src/a.rs".to_string(),
                vec![
                    sym("target", "target", 10),
                    sym("brand_new", "brand_new", 40),
                ],
            )]),
        };
        let files = vec![
            file(FileChangeStatus::Modified, vec![hunk(12, 1), hunk(40, 3)]),
            ChangedFile {
                path: "README.md".into(),
                old_path: None,
                status: FileChangeStatus::Modified,
                additions: Some(1),
                deletions: Some(0),
                binary: false,
                hunks: vec![hunk(1, 1)],
            },
        ];
        let repo = GitRepo::new(".");
        let impact = build_impact(
            &repo,
            ImpactRequest {
                sha: &sha,
                parent: Some(&parent),
                files: &files,
                depth: 2,
                include_inferred: true,
                patch_truncated: false,
            },
            Some(IndexContext {
                graph: &graph,
                run: &run,
            }),
            SymbolSupport::Available(&extractor),
        )
        .await
        .unwrap();

        assert_eq!(impact.run_id, Some(3));
        assert!(impact.unavailable_reason.is_none());
        let a = &impact.files[0];
        let by: HashMap<_, _> = a.symbols.iter().map(|s| (s.label.as_str(), s)).collect();
        assert_eq!(by["target"].change, SymbolChange::Modified);
        assert!(by["target"].in_index);
        assert_eq!(by["brand_new"].change, SymbolChange::Added);
        assert!(
            !by["brand_new"].in_index,
            "a symbol the index does not know yet"
        );
        assert_eq!(a.file_node_id.as_deref(), Some("file_a"));
        // The only dependent of the changed symbol is its caller.
        assert_eq!(
            impact
                .affected
                .iter()
                .map(|x| x.node.id.as_str())
                .collect::<Vec<_>>(),
            vec!["caller"]
        );
        // README.md is not a source file: no symbols, no failure.
        assert!(impact.files[1].symbols.is_empty());
    }

    #[tokio::test]
    async fn without_an_engine_the_impact_still_lists_files_and_says_why() {
        let files = vec![file(FileChangeStatus::Modified, vec![hunk(1, 1)])];
        let repo = GitRepo::new(".");
        let impact = build_impact(
            &repo,
            ImpactRequest {
                sha: &"c".repeat(40),
                parent: None,
                files: &files,
                depth: 2,
                include_inferred: true,
                patch_truncated: false,
            },
            None,
            SymbolSupport::Unavailable("Graphify is not installed".into()),
        )
        .await
        .unwrap();
        assert_eq!(
            impact.unavailable_reason.as_deref(),
            Some("Graphify is not installed")
        );
        assert_eq!(impact.files.len(), 1);
        assert!(impact.files[0].symbols.is_empty() && impact.affected.is_empty());
        assert_eq!(impact.run_id, None);
    }

    struct FailingExtractor;

    #[async_trait]
    impl SymbolExtractor for FailingExtractor {
        async fn symbols_at(
            &self,
            _repo: &GitRepo,
            _sha: &str,
            _paths: &[String],
        ) -> SymphonyResult<HashMap<String, Vec<FileSymbol>>> {
            Err(symphony_core::SymphonyError::IoError(
                "graphify timed out".into(),
            ))
        }
    }

    #[tokio::test]
    async fn a_failed_symbol_extraction_falls_back_to_the_file_list() {
        let files = vec![file(FileChangeStatus::Modified, vec![hunk(1, 1)])];
        let repo = GitRepo::new(".");
        let impact = build_impact(
            &repo,
            ImpactRequest {
                sha: &"c".repeat(40),
                parent: Some(&"b".repeat(40)),
                files: &files,
                depth: 2,
                include_inferred: true,
                patch_truncated: false,
            },
            None,
            SymbolSupport::Available(&FailingExtractor),
        )
        .await
        .unwrap();
        let reason = impact.unavailable_reason.unwrap();
        assert!(reason.contains("graphify timed out"), "{reason}");
        assert_eq!(impact.files.len(), 1);
        assert!(impact.files[0].symbols.is_empty());
    }

    #[tokio::test]
    async fn files_without_hunks_in_a_cut_short_patch_are_counted_as_not_analysed() {
        let sha = "c".repeat(40);
        let parent = "b".repeat(40);
        let extractor = FakeExtractor {
            pre: HashMap::new(),
            post: HashMap::from([("src/a.rs".to_string(), vec![sym("a", "a", 1)])]),
            parent: parent.clone(),
        };
        let files = vec![file(FileChangeStatus::Modified, Vec::new())];
        let repo = GitRepo::new(".");
        let impact = build_impact(
            &repo,
            ImpactRequest {
                sha: &sha,
                parent: Some(&parent),
                files: &files,
                depth: 2,
                include_inferred: true,
                patch_truncated: true,
            },
            None,
            SymbolSupport::Available(&extractor),
        )
        .await
        .unwrap();
        assert_eq!(impact.symbol_files_skipped, 1);
        assert!(impact.files[0].symbols.is_empty());
    }
}
