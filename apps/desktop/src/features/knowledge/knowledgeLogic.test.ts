import { describe, expect, it } from "vitest";
import type { CommitImpact, CommitSummary, GraphNode } from "@symphony/shared-types";
import { filterCommits, highlightFor, isTestFile, moduleForFile, shortSubject, worktreeLabel } from "./knowledgeLogic.js";

const node = (id: string, kind: GraphNode["kind"], extra: Partial<GraphNode> = {}): GraphNode => ({
  id,
  label: id,
  kind,
  file: null,
  line: null,
  module: null,
  degree: 0,
  members: 1,
  ...extra
});

const impact = (over: Partial<CommitImpact> = {}): CommitImpact => ({
  sha: "a".repeat(40),
  run_id: 1,
  indexed_sha: "b".repeat(40),
  unavailable_reason: null,
  files: [
    {
      path: "crates/core/src/lib.rs",
      status: "modified",
      file_node_id: "file_core",
      symbols: [
        { node_id: "fn_core", label: "run()", kind: "function", file: "crates/core/src/lib.rs", line: 10, change: "modified", precision: "start_to_next", in_index: true },
        { node_id: null, label: "fresh()", kind: "function", file: "crates/core/src/lib.rs", line: 40, change: "added", precision: "start_to_next", in_index: false }
      ],
      unattributed_hunks: 0
    }
  ],
  affected: [
    {
      node: node("fn_server", "function", { file: "crates/server/src/lib.rs", module: "crates/server" }),
      depth: 1,
      via_relation: "calls",
      from_node_id: "fn_core",
      confidence: "extracted"
    },
    {
      node: node("fn_cli", "function", { file: "crates/cli/src/main.rs", module: "crates/cli" }),
      depth: 2,
      via_relation: "calls",
      from_node_id: "fn_server",
      confidence: "inferred"
    }
  ],
  depth: 2,
  affected_truncated: false,
  symbol_files_skipped: 0,
  computed_at: "2026-01-01T00:00:00Z",
  ...over
});

describe("moduleForFile", () => {
  it("takes the longest module path that contains the file", () => {
    const modules = ["crates/core", "crates", "packages/ui", ".", "docs"];
    expect(moduleForFile("crates/core/src/lib.rs", modules)).toBe("crates/core");
    expect(moduleForFile("crates/other/x.rs", modules)).toBe("crates");
    expect(moduleForFile("README.md", modules)).toBe(".");
    expect(moduleForFile("docs/guide.md", modules)).toBe("docs");
    expect(moduleForFile("documents/a.md", ["docs"])).toBeNull();
  });
});

describe("highlightFor", () => {
  it("marks the modules of changed files and of affected nodes", () => {
    const nodes = ["crates/core", "crates/server", "crates/cli", "docs"].map((id) => node(id, "module"));
    const marks = highlightFor("modules", nodes, impact());
    expect(marks.get("crates/core")).toEqual({ emphasis: "changed", depth: 0 });
    expect(marks.get("crates/server")).toEqual({ emphasis: "affected", depth: 1 });
    expect(marks.get("crates/cli")).toEqual({ emphasis: "affected", depth: 2 });
    expect(marks.has("docs")).toBe(false);
  });

  it("changed wins over affected and the nearest depth wins among affected", () => {
    const nodes = ["crates/core", "crates/server"].map((id) => node(id, "module"));
    const marks = highlightFor(
      "modules",
      nodes,
      impact({
        affected: [
          { node: node("a", "function", { module: "crates/core" }), depth: 1, via_relation: "calls", from_node_id: "x", confidence: "extracted" },
          { node: node("b", "function", { module: "crates/server" }), depth: 3, via_relation: "calls", from_node_id: "x", confidence: "extracted" },
          { node: node("c", "function", { module: "crates/server" }), depth: 2, via_relation: "calls", from_node_id: "x", confidence: "extracted" }
        ]
      })
    );
    expect(marks.get("crates/core")?.emphasis).toBe("changed");
    expect(marks.get("crates/server")).toEqual({ emphasis: "affected", depth: 2 });
  });

  it("matches files by path at the file level", () => {
    const nodes = [
      node("file_core", "file", { file: "crates/core/src/lib.rs" }),
      node("file_server", "file", { file: "crates/server/src/lib.rs" }),
      node("file_other", "file", { file: "crates/other/src/lib.rs" })
    ];
    const marks = highlightFor("files", nodes, impact());
    expect(marks.get("file_core")?.emphasis).toBe("changed");
    expect(marks.get("file_server")).toEqual({ emphasis: "affected", depth: 1 });
    expect(marks.has("file_other")).toBe(false);
  });

  it("uses node ids at the symbol level and ignores symbols that are not on screen", () => {
    const nodes = [node("file_core", "file"), node("fn_core", "function"), node("fn_server", "function")];
    const marks = highlightFor("symbols", nodes, impact());
    expect([...marks.keys()].sort()).toEqual(["file_core", "fn_core", "fn_server"]);
    expect(marks.get("fn_core")?.emphasis).toBe("changed");
    expect(highlightFor("symbols", nodes, null).size).toBe(0);
  });
});

describe("timeline helpers", () => {
  const commit = (over: Partial<CommitSummary>): CommitSummary => ({
    sha: "a".repeat(40),
    short_sha: "aaaaaaaa",
    parents: [],
    author_name: "Ada",
    authored_at: "2026-01-01T00:00:00Z",
    committed_at: "2026-01-01T00:00:00Z",
    subject: "Fix agent lifecycle",
    is_merge: false,
    on_default_branch: true,
    unpushed: false,
    refs: ["main"],
    worktrees: [{ worktree_id: "wt_1", branch: "main", is_head: true }],
    ...over
  });

  it("filters by text, worktree and push state", () => {
    const a = commit({});
    const b = commit({ sha: "b".repeat(40), subject: "Add provider routing", refs: [], unpushed: true, worktrees: [{ worktree_id: "wt_2", branch: "agent/x", is_head: true }] });
    const all = [a, b];
    expect(filterCommits(all, { text: "", worktreeId: null, onlyUnpushed: false })).toHaveLength(2);
    expect(filterCommits(all, { text: "routing", worktreeId: null, onlyUnpushed: false })).toEqual([b]);
    expect(filterCommits(all, { text: "agent/x", worktreeId: null, onlyUnpushed: false })).toEqual([b]);
    expect(filterCommits(all, { text: "", worktreeId: "wt_1", onlyUnpushed: false })).toEqual([a]);
    expect(filterCommits(all, { text: "", worktreeId: null, onlyUnpushed: true })).toEqual([b]);
  });

  it("labels worktrees and recognises test files", () => {
    expect(worktreeLabel({ branch: "feat/x", head_short_sha: "abc", path: "/r/wt" })).toBe("feat/x");
    expect(worktreeLabel({ branch: null, head_short_sha: "abc12345", path: "/r/wt" })).toBe("detached abc12345");
    expect(worktreeLabel({ branch: null, head_short_sha: "abc12345", path: "/r/wt" }, "fristående")).toBe("fristående abc12345");
    expect(worktreeLabel({ branch: null, head_short_sha: null, path: "C:\\repo\\wt-a" })).toBe("wt-a");
    for (const yes of ["src/a.test.ts", "tests/api.rs", "crates/x/tests/it.rs", "pkg/foo_test.go", "e2e/flows.spec.ts", "a/__tests__/b.js", "test_foo.py", "pkg/test_bar.py"]) {
      expect(isTestFile(yes), yes).toBe(true);
    }
    for (const no of ["src/latest.ts", "src/contest/a.rs", "README.md", "src/attest_foo.py"]) expect(isTestFile(no), no).toBe(false);
    expect(shortSubject("a".repeat(120), 20)).toHaveLength(20);
    expect(shortSubject("first\nsecond")).toBe("first");
  });
});
