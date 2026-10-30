import type {
  CommitImpact,
  CommitSummary,
  GraphLevel,
  GraphNode,
  KnowledgeWorktree,
  NodeKind
} from "@symphony/shared-types";

/** How a node of the displayed graph relates to the selected commit. */
export type Emphasis = "changed" | "affected";

export interface Mark {
  emphasis: Emphasis;
  /** Hops from the nearest changed node (0 for changed ones). */
  depth: number;
}

/**
 * The module a file belongs to, among the modules that are displayed: the longest module path
 * that is a prefix of the file path (the root module `.` takes top-level files).
 */
export function moduleForFile(file: string, moduleIds: ReadonlyArray<string>): string | null {
  let best: string | null = null;
  for (const id of moduleIds) {
    const matches = id === "." ? !file.includes("/") : file === id || file.startsWith(`${id}/`);
    if (matches && (best === null || id.length > best.length)) best = id;
  }
  return best;
}

/**
 * Which nodes of the graph that is on screen a commit marks. The commit's impact is expressed in
 * files and symbols; the module level lifts it to the modules that contain them, the file level
 * to files, and the symbol level uses node ids directly. A node that is both changed and
 * affected counts as changed.
 */
export function highlightFor(
  level: GraphLevel,
  nodes: ReadonlyArray<GraphNode>,
  impact: CommitImpact | null | undefined
): Map<string, Mark> {
  const marks = new Map<string, Mark>();
  if (!impact) return marks;
  const set = (id: string, mark: Mark) => {
    const existing = marks.get(id);
    if (!existing || (existing.emphasis === "affected" && mark.emphasis === "changed") || (existing.emphasis === mark.emphasis && mark.depth < existing.depth)) {
      marks.set(id, mark);
    }
  };
  const ids = new Set(nodes.map((n) => n.id));

  if (level === "modules") {
    const moduleIds = nodes.map((n) => n.id);
    for (const file of impact.files) {
      const module = moduleForFile(file.path, moduleIds);
      if (module) set(module, { emphasis: "changed", depth: 0 });
    }
    for (const affected of impact.affected) {
      const module = affected.node.module;
      if (module && ids.has(module)) set(module, { emphasis: "affected", depth: affected.depth });
    }
    return marks;
  }

  if (level === "files") {
    const byFile = new Map<string, string>();
    for (const node of nodes) if (node.file && node.kind === "file") byFile.set(node.file, node.id);
    for (const file of impact.files) {
      const id = byFile.get(file.path);
      if (id) set(id, { emphasis: "changed", depth: 0 });
    }
    for (const affected of impact.affected) {
      const id = affected.node.file ? byFile.get(affected.node.file) : undefined;
      if (id) set(id, { emphasis: "affected", depth: affected.depth });
    }
    return marks;
  }

  for (const file of impact.files) {
    if (file.file_node_id && ids.has(file.file_node_id)) set(file.file_node_id, { emphasis: "changed", depth: 0 });
    for (const symbol of file.symbols) {
      if (symbol.node_id && ids.has(symbol.node_id)) set(symbol.node_id, { emphasis: "changed", depth: 0 });
    }
  }
  for (const affected of impact.affected) {
    if (ids.has(affected.node.id)) set(affected.node.id, { emphasis: "affected", depth: affected.depth });
  }
  return marks;
}

export interface CommitFilter {
  text: string;
  worktreeId: string | null;
  onlyUnpushed: boolean;
}

/** Filter the timeline; the text matches subject, author, short id and branch names. */
export function filterCommits(commits: ReadonlyArray<CommitSummary>, filter: CommitFilter): CommitSummary[] {
  const needle = filter.text.trim().toLowerCase();
  return commits.filter((commit) => {
    if (filter.worktreeId && !commit.worktrees.some((w) => w.worktree_id === filter.worktreeId)) return false;
    if (filter.onlyUnpushed && commit.unpushed !== true) return false;
    if (!needle) return true;
    const haystack = [commit.subject, commit.author_name, commit.sha, ...commit.refs, ...commit.worktrees.map((w) => w.branch ?? "")]
      .join("\n")
      .toLowerCase();
    return haystack.includes(needle);
  });
}

/** A short label for a worktree: its branch, or `detached <sha>`. */
export function worktreeLabel(worktree: Pick<KnowledgeWorktree, "branch" | "head_short_sha" | "path">, detached = "detached"): string {
  if (worktree.branch) return worktree.branch;
  if (worktree.head_short_sha) return `${detached} ${worktree.head_short_sha}`;
  return worktree.path.split(/[\\/]/).filter(Boolean).pop() ?? worktree.path;
}

const TEST_PATH = /(^|\/)(tests?|__tests__|e2e|spec)(\/|$)|(\.|_)(test|spec)\.[a-z0-9]+$|_test\.(go|py|rs)$|(^|\/)test_[^/]+\.py$/i;

/** Whether a repository path is a test file by common naming conventions. */
export function isTestFile(path: string): boolean {
  return TEST_PATH.test(path.replace(/\\/g, "/"));
}

/** Kinds shown as symbols in lists and the legend (everything but file/package/module/dependency). */
export function isSymbolKind(kind: NodeKind): boolean {
  return kind === "type" || kind === "function" || kind === "method";
}

/** Display name of a node: methods and functions drop the trailing `()` noise only for sorting. */
export function nodeTitle(node: Pick<GraphNode, "label" | "file" | "kind">): string {
  return node.label;
}

/** `path:line` for a node or evidence, with the line omitted when unknown. */
export function locationLabel(file: string | null | undefined, line: number | null | undefined): string {
  if (!file) return "";
  return line ? `${file}:${line}` : file;
}

/** The first line of a commit message. */
export function shortSubject(subject: string, max = 90): string {
  const first = subject.split("\n", 1)[0] ?? "";
  return first.length > max ? `${first.slice(0, max - 1)}…` : first;
}
