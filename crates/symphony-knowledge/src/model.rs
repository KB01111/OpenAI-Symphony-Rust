//! Data types of the Knowledge Base: what the HTTP API returns and what the modules exchange.
//!
//! Every value here is derived from the repository (git) or from an engine's output; nothing is
//! synthesised. Where a value is inferred rather than read, the type carries the evidence level
//! (`Confidence`, `Precision`) so the UI can say so.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use symphony_core::{ProviderKind, TaskStatus};
use utoipa::ToSchema;

// ---------------------------------------------------------------------------------------------
// Engines
// ---------------------------------------------------------------------------------------------

/// A tool the Knowledge Base can use (`git`, `graphify`, `serena`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EngineStatus {
    /// `git`, `graphify` or `serena`.
    pub id: String,
    pub name: String,
    pub available: bool,
    /// What the Knowledge Base cannot do without it.
    pub required: bool,
    pub version: Option<String>,
    /// The executable that was found (or the configured command that was not).
    pub command: Option<String>,
    /// A command the user can run to install it; Symphony never installs anything itself.
    pub install_hint: Option<String>,
    /// Why it is unavailable, or a licence/usage note.
    pub detail: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Worktrees and commits
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeKind {
    /// The repository's primary checkout.
    Main,
    /// A worktree Symphony allocated for a task.
    Symphony,
    /// A linked worktree created by something else (a developer, another tool).
    External,
}

/// The Symphony task that owns a worktree, when there is one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WorktreeTaskLink {
    pub task_id: String,
    pub issue_identifier: String,
    pub issue_title: String,
    pub task_status: TaskStatus,
    pub provider: ProviderKind,
    pub active_session_id: Option<String>,
}

/// One git worktree of the repository (`git worktree list`), with live state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct KnowledgeWorktree {
    /// Stable id derived from the path.
    pub id: String,
    pub path: String,
    pub kind: WorktreeKind,
    pub branch: Option<String>,
    pub detached: bool,
    pub head_sha: Option<String>,
    pub head_short_sha: Option<String>,
    pub head_subject: Option<String>,
    pub head_committed_at: Option<DateTime<Utc>>,
    /// The directory exists on disk (a prunable worktree may have been deleted).
    pub exists: bool,
    pub dirty: bool,
    pub changed_files: u32,
    pub locked: bool,
    pub prunable: bool,
    /// Commits reachable from this worktree's HEAD that are not on the default branch.
    pub ahead_of_default: Option<u32>,
    pub task: Option<WorktreeTaskLink>,
}

/// Which worktree contains a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CommitWorktreeRef {
    pub worktree_id: String,
    pub branch: Option<String>,
    /// The commit is this worktree's HEAD (not just in its history).
    pub is_head: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CommitSummary {
    pub sha: String,
    pub short_sha: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub authored_at: DateTime<Utc>,
    pub committed_at: DateTime<Utc>,
    pub subject: String,
    pub is_merge: bool,
    /// Reachable from the default branch.
    pub on_default_branch: bool,
    /// Not on any remote-tracking ref. `None` when the repository has no remote refs at all, so
    /// "unpushed" would be meaningless.
    pub unpushed: Option<bool>,
    /// Branches and tags pointing at this commit.
    pub refs: Vec<String>,
    /// Every worktree whose HEAD history contains this commit; one entry per worktree, one row per SHA.
    pub worktrees: Vec<CommitWorktreeRef>,
}

/// Uncommitted work of a worktree, shown next to the commits (it has no SHA).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkingTreeChange {
    pub worktree_id: String,
    pub branch: Option<String>,
    pub changed_files: u32,
    /// The first changed paths.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CommitTimeline {
    pub commits: Vec<CommitSummary>,
    pub working_trees: Vec<WorkingTreeChange>,
    pub worktrees_total: u32,
    /// There are more commits than `limit`.
    pub truncated: bool,
    pub has_remote_refs: bool,
    pub default_branch: String,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

/// A changed line range. Zero-length ranges are pure insertions (`new_lines == 0` is a deletion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileChangeStatus,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub binary: bool,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CommitDetail {
    pub commit: CommitSummary,
    pub body: String,
    pub files: Vec<ChangedFile>,
    pub additions: u32,
    pub deletions: u32,
    pub files_truncated: bool,
    /// The commit the changes are relative to (the first parent); `None` for a root commit.
    pub diff_base: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A crate / npm package (from its manifest).
    Package,
    /// A directory-level grouping computed from file paths.
    Module,
    File,
    /// A struct, class, enum, trait, interface or type alias.
    Type,
    Function,
    Method,
    /// An external dependency (a crate or package outside the repository).
    Dependency,
    Other,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Module => "module",
            Self::File => "file",
            Self::Type => "type",
            Self::Function => "function",
            Self::Method => "method",
            Self::Dependency => "dependency",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "package" => Self::Package,
            "module" => Self::Module,
            "file" => Self::File,
            "type" => Self::Type,
            "function" => Self::Function,
            "method" => Self::Method,
            "dependency" => Self::Dependency,
            _ => Self::Other,
        }
    }

    /// A node that owns a line range inside a file.
    pub fn is_symbol(self) -> bool {
        matches!(self, Self::Type | Self::Function | Self::Method)
    }
}

/// How the engine came to an edge. `Extracted` was read from the source; the other two are
/// inferred, and the UI draws them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Extracted,
    Inferred,
    Ambiguous,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extracted => "extracted",
            Self::Inferred => "inferred",
            Self::Ambiguous => "ambiguous",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "extracted" => Self::Extracted,
            "ambiguous" => Self::Ambiguous,
            _ => Self::Inferred,
        }
    }

    /// Higher is stronger evidence.
    pub fn rank(self) -> u8 {
        match self {
            Self::Extracted => 2,
            Self::Inferred => 1,
            Self::Ambiguous => 0,
        }
    }

    /// The weaker of two confidences (a path is only as strong as its weakest edge).
    pub fn weakest(self, other: Self) -> Self {
        if self.rank() <= other.rank() {
            self
        } else {
            other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    /// Repository-relative path, `None` for externals and aggregated modules.
    pub file: Option<String>,
    pub line: Option<u32>,
    /// The package or directory the node belongs to.
    pub module: Option<String>,
    pub degree: u32,
    /// For aggregated nodes: how many underlying nodes they stand for.
    pub members: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: Confidence,
    /// Line of the occurrence (for example the call site) in the source node's file.
    pub line: Option<u32>,
    /// For aggregated edges: how many underlying edges they stand for.
    pub weight: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GraphLevel {
    /// One node per package / directory module, edges aggregated between them.
    Modules,
    /// Files and the import relations between them.
    Files,
    /// Symbols (types, functions, methods) with their calls and references.
    Symbols,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GraphView {
    pub run_id: i64,
    pub level: GraphLevel,
    /// The module or node the view is centred on, when one was requested.
    pub focus: Option<String>,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Counts before the size limit was applied.
    pub total_nodes: u32,
    pub total_edges: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct NodeNeighbor {
    pub node: GraphNode,
    pub relation: String,
    pub confidence: Confidence,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NodeDetail {
    pub node: GraphNode,
    /// What this node uses (outgoing dependency edges).
    pub dependencies: Vec<NodeNeighbor>,
    /// What uses this node (incoming dependency edges).
    pub references: Vec<NodeNeighbor>,
    /// Members of a file / type (contained symbols).
    pub members: Vec<GraphNode>,
    pub dependencies_total: u32,
    pub references_total: u32,
    /// Commits that touched the node's file, newest first (from git, not from the index).
    pub recent_commits: Vec<CommitSummary>,
}

/// One place that uses a symbol, as a language server reports it (Serena).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SymbolReference {
    pub file: String,
    /// The symbol that contains the use, for example `impl Orchestrator/list_worktrees`.
    pub name_path: String,
    pub kind: String,
    /// Range of the containing symbol (1-based, inclusive).
    pub start_line: u32,
    pub end_line: u32,
    /// The line of the use itself.
    pub reference_line: Option<u32>,
    /// A few lines around the use.
    pub snippet: String,
}

/// Precise references of a symbol from the optional language-server navigator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SymbolReferences {
    pub node_id: String,
    /// `false` when the navigator could not answer; `reason` says why.
    pub available: bool,
    pub reason: Option<String>,
    pub references: Vec<SymbolReference>,
    pub total: u32,
    pub truncated: bool,
    /// The exact range of the definition (1-based, inclusive), including its doc comment.
    pub definition_start_line: Option<u32>,
    pub definition_end_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SearchHit {
    pub node: GraphNode,
}

// ---------------------------------------------------------------------------------------------
// Index runs and overview
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexStatus {
    Running,
    Completed,
    Failed,
}

impl IndexStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LanguageStat {
    pub language: String,
    pub files: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct IndexRunSummary {
    pub id: i64,
    pub project_id: String,
    /// The branch or worktree the indexed commit came from.
    pub ref_name: String,
    pub commit_sha: String,
    pub engine: String,
    pub engine_version: String,
    pub status: IndexStatus,
    pub node_count: u32,
    pub edge_count: u32,
    pub file_count: u32,
    pub languages: Vec<LanguageStat>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ModuleStat {
    pub module: String,
    pub files: u32,
    pub symbols: u32,
    pub incoming: u32,
    pub outgoing: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HealthSeverity {
    Info,
    Warning,
    Problem,
}

/// One observation about the repository or the index, each backed by a measured value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthSignal {
    pub code: String,
    pub severity: HealthSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct KnowledgeOverview {
    pub project_id: String,
    pub default_branch: String,
    /// The default branch tip right now.
    pub default_branch_sha: Option<String>,
    /// The newest completed index, if any.
    pub index: Option<IndexRunSummary>,
    /// An index that is running now, or the last one if it failed.
    pub active_run: Option<IndexRunSummary>,
    /// The index is older than the commit it was built from's branch tip.
    pub index_stale: bool,
    pub engines: Vec<EngineStatus>,
    pub worktrees_total: u32,
    pub worktrees_dirty: u32,
    /// Types, functions and methods in the index (0 without an index).
    pub symbol_count: u32,
    pub top_modules: Vec<ModuleStat>,
    pub hubs: Vec<GraphNode>,
    pub health: Vec<HealthSignal>,
    pub generated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------------------------
// Commit impact
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SymbolChange {
    Added,
    Modified,
    Removed,
}

/// How exactly a changed line range was mapped onto a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    /// The engine reports only where a symbol starts, so it is taken to extend to the next symbol
    /// in the file: lines between two symbols (imports, constants, comments) are attributed to the
    /// preceding one.
    StartToNext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ChangedSymbol {
    /// The node id in the index, when the symbol still exists there.
    pub node_id: Option<String>,
    pub label: String,
    pub kind: NodeKind,
    pub file: String,
    pub line: u32,
    pub change: SymbolChange,
    pub precision: Precision,
    /// `node_id` was found in the indexed graph (otherwise the symbol is new or was removed since).
    pub in_index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ImpactFile {
    pub path: String,
    pub status: FileChangeStatus,
    pub file_node_id: Option<String>,
    pub symbols: Vec<ChangedSymbol>,
    /// Changed lines that fall outside every known symbol (imports, constants, comments).
    pub unattributed_hunks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AffectedNode {
    pub node: GraphNode,
    /// Hops from the nearest changed node.
    pub depth: u32,
    /// The relation of the last hop (for example `calls`).
    pub via_relation: String,
    /// The node it was reached from.
    pub from_node_id: String,
    /// The weakest edge on the path.
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CommitImpact {
    pub sha: String,
    /// The index the affected set was computed on, if there is one.
    pub run_id: Option<i64>,
    pub indexed_sha: Option<String>,
    /// Why the symbol-level part is missing, when it is.
    pub unavailable_reason: Option<String>,
    pub files: Vec<ImpactFile>,
    pub affected: Vec<AffectedNode>,
    pub depth: u32,
    pub affected_truncated: bool,
    /// Source files beyond the per-commit limit that were not looked at symbol by symbol.
    pub symbol_files_skipped: u32,
    pub computed_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------------------------
// AI analysis and chat
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    File,
    Symbol,
    Commit,
    Hunk,
}

/// A piece of repository material the model was given and may cite. Only ids that were in the
/// prompt survive validation, so every citation resolves to something real.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EvidenceRef {
    pub id: String,
    pub kind: EvidenceKind,
    pub label: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub node_id: Option<String>,
    pub sha: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AnalysisFinding {
    pub title: String,
    pub detail: String,
    pub severity: FindingSeverity,
    /// Ids from `CommitAnalysis::evidence`; never empty after validation.
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TestSuggestion {
    pub title: String,
    pub rationale: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CommitAnalysisResult {
    pub summary: String,
    pub what_changed: Vec<AnalysisFinding>,
    pub risks: Vec<AnalysisFinding>,
    pub security: Vec<AnalysisFinding>,
    pub regressions: Vec<AnalysisFinding>,
    pub tests: Vec<TestSuggestion>,
    pub recommendations: Vec<AnalysisFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Completed,
    Failed,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CommitAnalysis {
    pub id: i64,
    pub project_id: String,
    pub sha: String,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub prompt_version: u32,
    pub status: JobStatus,
    pub result: Option<CommitAnalysisResult>,
    /// Everything the model was allowed to cite.
    pub evidence: Vec<EvidenceRef>,
    /// Findings the model produced that cited nothing the prompt contained; they are not shown.
    pub dropped_findings: u32,
    pub error: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value == "user" {
            Self::User
        } else {
            Self::Assistant
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ChatMessage {
    pub id: i64,
    pub project_id: String,
    pub role: ChatRole,
    pub content: String,
    pub evidence: Vec<EvidenceRef>,
    pub status: JobStatus,
    pub provider: Option<ProviderKind>,
    pub model: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}
