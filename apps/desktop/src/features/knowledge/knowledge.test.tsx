import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphEdge, GraphNode } from "@symphony/shared-types";
import KnowledgePage from "./KnowledgePage.js";
import { createMockRuntime, renderWithProviders, type MockHandler } from "../../test/renderWithProviders.js";
import { project, snapshot } from "../../test/fixtures.js";
import { useUiStore } from "../../store/uiStore.js";
import type { Mark } from "./knowledgeLogic.js";

// jsdom has no WebGL: the canvas is replaced by a stub that exposes what it was asked to draw.
const canvas = vi.hoisted(() => ({ last: null as null | { nodes: GraphNode[]; edges: GraphEdge[]; marks: Map<string, Mark> } }));
vi.mock("./GraphCanvas.js", () => ({
  default: (props: { nodes: GraphNode[]; edges: GraphEdge[]; marks: Map<string, Mark>; onSelect: (id: string | null) => void }) => {
    canvas.last = { nodes: props.nodes, edges: props.edges, marks: props.marks };
    return (
      <div data-testid="graph-canvas">
        {props.nodes.map((node) => (
          <button key={node.id} type="button" onClick={() => props.onSelect(node.id)}>
            canvas:{node.label}
          </button>
        ))}
      </div>
    );
  }
}));

const SHA = "a".repeat(40);
const PARENT = "b".repeat(40);

const node = (over: Partial<GraphNode> & Pick<GraphNode, "id" | "label" | "kind">): GraphNode => ({
  file: null,
  line: null,
  module: null,
  degree: 0,
  members: 1,
  ...over
});

const summary = (over = {}) => ({
  sha: SHA,
  short_sha: SHA.slice(0, 8),
  parents: [PARENT],
  author_name: "Ada",
  authored_at: "2026-10-30T10:00:00Z",
  committed_at: "2026-10-30T10:00:00Z",
  subject: "Fix agent lifecycle",
  is_merge: false,
  on_default_branch: false,
  unpushed: true,
  refs: [],
  worktrees: [{ worktree_id: "wt_agent", branch: "agent/task-1", is_head: true }],
  ...over
});

const engines = [
  { id: "git", name: "Git", available: true, required: true, version: "2.54.0", command: "git", install_hint: null, detail: null },
  { id: "graphify", name: "Graphify", available: true, required: true, version: "0.9.97", command: "graphify", install_hint: "uv tool install graphifyy==0.9.97", detail: null },
  { id: "serena", name: "Serena", available: false, required: false, version: null, command: null, install_hint: "uv tool install -p 3.13 serena-agent && serena init", detail: "GPL-3.0" }
];

const indexRun = {
  id: 1,
  project_id: "p1",
  ref_name: "main",
  commit_sha: PARENT,
  engine: "graphify",
  engine_version: "0.9.97",
  status: "completed",
  node_count: 120,
  edge_count: 340,
  file_count: 24,
  languages: [
    { language: "Rust", files: 18 },
    { language: "TypeScript", files: 6 }
  ],
  error: null,
  started_at: "2026-10-30T09:00:00Z",
  finished_at: "2026-10-30T09:01:00Z",
  duration_ms: 12400
};

const overview = (over = {}) => ({
  project_id: "p1",
  default_branch: "main",
  default_branch_sha: PARENT,
  index: indexRun,
  active_run: null,
  index_stale: false,
  engines,
  worktrees_total: 2,
  worktrees_dirty: 1,
  symbol_count: 77,
  top_modules: [{ module: "crates/core", files: 10, symbols: 40, incoming: 12, outgoing: 3 }],
  hubs: [node({ id: "fn_run", label: "run()", kind: "function", file: "crates/core/src/lib.rs", line: 10, degree: 9 })],
  health: [{ code: "dirty_worktrees", severity: "info", message: "1 of 2 worktree(s) have uncommitted changes." }],
  generated_at: "2026-10-30T10:00:00Z",
  ...over
});

const impact = {
  sha: SHA,
  run_id: 1,
  indexed_sha: PARENT,
  unavailable_reason: null,
  files: [
    {
      path: "crates/core/src/lib.rs",
      status: "modified",
      file_node_id: "file_core",
      symbols: [
        { node_id: "fn_run", label: "run()", kind: "function", file: "crates/core/src/lib.rs", line: 10, change: "modified", precision: "start_to_next", in_index: true }
      ],
      unattributed_hunks: 0
    },
    { path: "crates/core/tests/it.rs", status: "added", file_node_id: null, symbols: [], unattributed_hunks: 0 }
  ],
  affected: [
    {
      node: node({ id: "fn_serve", label: "serve()", kind: "function", file: "crates/server/src/lib.rs", line: 55, module: "crates/server", degree: 4 }),
      depth: 1,
      via_relation: "calls",
      from_node_id: "fn_run",
      confidence: "extracted"
    }
  ],
  depth: 2,
  affected_truncated: false,
  symbol_files_skipped: 0,
  computed_at: "2026-10-30T10:00:00Z"
};

function runtime(extra: Record<string, MockHandler> = {}) {
  return createMockRuntime({
    "GET /projects": () => [project({ id: "p1", name: "Demo repo" })],
    "GET /state": () => snapshot(),
    "GET /projects/:id/knowledge": () => overview(),
    "GET /projects/:id/knowledge/worktrees": () => [
      {
        id: "wt_main",
        path: "/repo",
        kind: "main",
        branch: "main",
        detached: false,
        head_sha: PARENT,
        head_short_sha: PARENT.slice(0, 8),
        head_subject: "Initial commit",
        head_committed_at: "2026-10-29T10:00:00Z",
        exists: true,
        dirty: false,
        changed_files: 0,
        locked: false,
        prunable: false,
        ahead_of_default: 0,
        task: null
      },
      {
        id: "wt_agent",
        path: "/repo/.wt/agent",
        kind: "symphony",
        branch: "agent/task-1",
        detached: false,
        head_sha: SHA,
        head_short_sha: SHA.slice(0, 8),
        head_subject: "Fix agent lifecycle",
        head_committed_at: "2026-10-30T10:00:00Z",
        exists: true,
        dirty: true,
        changed_files: 3,
        locked: false,
        prunable: false,
        ahead_of_default: 1,
        task: { task_id: "t1", issue_identifier: "SYM-7", issue_title: "Lifecycle", task_status: "running", provider: "codex", active_session_id: "s1" }
      }
    ],
    "GET /projects/:id/knowledge/commits": () => ({
      commits: [
        summary(),
        summary({
          sha: PARENT,
          short_sha: PARENT.slice(0, 8),
          parents: [],
          subject: "Initial commit",
          unpushed: false,
          on_default_branch: true,
          refs: ["main"],
          worktrees: [
            { worktree_id: "wt_main", branch: "main", is_head: true },
            { worktree_id: "wt_agent", branch: "agent/task-1", is_head: false }
          ]
        })
      ],
      working_trees: [{ worktree_id: "wt_agent", branch: "agent/task-1", changed_files: 3, files: ["a.rs", "b.rs", "c.rs"] }],
      worktrees_total: 2,
      truncated: false,
      has_remote_refs: true,
      default_branch: "main",
      generated_at: "2026-10-30T10:00:00Z"
    }),
    "GET /projects/:id/knowledge/commits/:sha": () => ({
      commit: summary(),
      body: "",
      files: [
        { path: "crates/core/src/lib.rs", old_path: null, status: "modified", additions: 4, deletions: 1, binary: false, hunks: [{ old_start: 10, old_lines: 1, new_start: 10, new_lines: 4 }] },
        { path: "crates/core/tests/it.rs", old_path: null, status: "added", additions: 20, deletions: 0, binary: false, hunks: [] }
      ],
      additions: 24,
      deletions: 1,
      files_truncated: false,
      diff_base: PARENT
    }),
    "GET /projects/:id/knowledge/commits/:sha/impact": () => impact,
    "GET /projects/:id/knowledge/commits/:sha/diff": () => ({
      sha: SHA,
      diff: "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old line\n+new line\n",
      truncated: false
    }),
    "GET /projects/:id/knowledge/commits/:sha/analysis": () => ({ analysis: null }),
    "GET /projects/:id/knowledge/graph": ({ query }) => ({
      run_id: 1,
      level: query.get("level") ?? "modules",
      focus: query.get("focus"),
      nodes: [
        node({ id: "crates/core", label: "core", kind: "module", module: "crates/core", members: 10, degree: 12 }),
        node({ id: "crates/server", label: "server", kind: "module", module: "crates/server", members: 6, degree: 5 })
      ],
      edges: [{ source: "crates/server", target: "crates/core", relation: "depends", confidence: "extracted", line: null, weight: 12 }],
      total_nodes: 2,
      total_edges: 1,
      truncated: false
    }),
    "GET /projects/:id/knowledge/chat": () => [],
    ...extra
  });
}

function open(route: string, rt = runtime()) {
  const view = renderWithProviders(<KnowledgePage />, { runtime: rt, route, path: "/knowledge" });
  return { rt, ...view };
}

describe("Repository Knowledge Base", () => {
  beforeEach(() => {
    canvas.last = null;
    useUiStore.setState({ activeProjectId: "p1", locale: "en" });
  });

  it("shows the index, engines with install commands, health and modules", async () => {
    open("/knowledge");
    expect(await screen.findByRole("heading", { name: "Repository Knowledge Base" })).toBeInTheDocument();
    expect(await screen.findByText("Indexed")).toBeInTheDocument();
    expect(await screen.findByText("77")).toBeInTheDocument();
    expect(screen.getByText("1 of 2 worktree(s) have uncommitted changes.")).toBeInTheDocument();
    // An engine that is not installed says how to install it; Symphony never does it itself.
    expect(screen.getByText("uv tool install -p 3.13 serena-agent && serena init")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy install command" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "crates/core" })).toBeInTheDocument();
  });

  it("lists worktrees with their agent and uncommitted work", async () => {
    open("/knowledge?tab=worktrees");
    // The branch also appears on the commit strip, so look for it in the table.
    expect((await screen.findAllByText("agent/task-1")).length).toBeGreaterThan(0);
    expect(screen.getByText("3 changes")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /SYM-7/ })).toBeInTheDocument();
    expect(screen.getByText("Symphony")).toBeInTheDocument();
    expect(screen.getByText("Main")).toBeInTheDocument();
    expect(screen.getByText("+1")).toBeInTheDocument();
  });

  it("shows one timeline row per commit and the uncommitted work of a worktree", async () => {
    open("/knowledge?tab=commits");
    expect(await screen.findByText("3 uncommitted files in agent/task-1")).toBeInTheDocument();
    const list = await screen.findByRole("list", { name: "Commits across all worktrees" });
    expect(within(list).getAllByText("Initial commit")).toHaveLength(1);
    expect(screen.getByText("Select a commit to see what it changed and what could break.")).toBeInTheDocument();
  });

  it("selecting a commit shows what changed and what could break, and highlights it in the graph", async () => {
    const { rt } = open("/knowledge?tab=commits");
    fireEvent.click((await screen.findAllByText("Fix agent lifecycle"))[0]!);

    // The changed symbol and its dependents come from the impact endpoint.
    expect(await screen.findByText("run()")).toBeInTheDocument();
    expect(await screen.findByText("serve()")).toBeInTheDocument();
    expect(screen.getByText("Modified")).toBeInTheDocument();
    expect(screen.getByText(/Symbol ranges are approximate/)).toBeInTheDocument();
    expect(rt.calls.some((c) => c.path.endsWith(`/commits/${SHA}/impact`))).toBe(true);

    fireEvent.click(screen.getAllByRole("button", { name: "Show in graph" })[0]!);
    // The module level marks the module of the changed file and the module that depends on it.
    await waitFor(() => expect(canvas.last?.marks.get("crates/core")).toEqual({ emphasis: "changed", depth: 0 }));
    expect(canvas.last?.marks.get("crates/server")).toEqual({ emphasis: "affected", depth: 1 });
    expect(await screen.findByText(/1 changed · 1 affected/)).toBeInTheDocument();
  });

  it("the Tests tab lists test files that changed in the commit", async () => {
    open(`/knowledge?tab=commits&commit=${SHA}`);
    fireEvent.click(await screen.findByRole("tab", { name: "Tests" }));
    expect(await screen.findByText("crates/core/tests/it.rs")).toBeInTheDocument();
  });

  it("shows the diff of the selected commit", async () => {
    open(`/knowledge?tab=commits&commit=${SHA}`);
    fireEvent.click(await screen.findByRole("tab", { name: "Diff" }));
    expect(await screen.findByText(/new line/)).toBeInTheDocument();
  });

  it("runs the AI review and shows findings with the evidence they cite", async () => {
    let analysis: unknown = null;
    const rt = runtime({
      "GET /projects/:id/knowledge/commits/:sha/analysis": () => ({ analysis }),
      "POST /projects/:id/knowledge/commits/:sha/analysis": () => {
        analysis = {
          id: 1,
          project_id: "p1",
          sha: SHA,
          provider: "codex",
          model: "gpt-test",
          prompt_version: 1,
          status: "completed",
          result: {
            summary: "Changes the agent lifecycle.",
            what_changed: [{ title: "run() reworked", detail: "It now stops sessions.", severity: "info", evidence: ["s1"] }],
            risks: [{ title: "Callers may hang", detail: "serve() waits on run().", severity: "high", evidence: ["s1", "a1"] }],
            security: [],
            regressions: [],
            recommendations: [],
            tests: [{ title: "Cancel during a turn", rationale: "Covers the new stop path.", evidence: ["f1"] }]
          },
          evidence: [
            { id: "s1", kind: "symbol", label: "run() (modified)", file: "crates/core/src/lib.rs", line: 10, node_id: "fn_run", sha: SHA },
            { id: "a1", kind: "symbol", label: "serve() (affected)", file: "crates/server/src/lib.rs", line: 55, node_id: "fn_serve", sha: null },
            { id: "f1", kind: "file", label: "crates/core/src/lib.rs", file: "crates/core/src/lib.rs", line: null, node_id: null, sha: SHA }
          ],
          dropped_findings: 2,
          error: null,
          input_tokens: 1200,
          output_tokens: 300,
          created_at: "2026-10-30T10:05:00Z",
          finished_at: "2026-10-30T10:05:20Z"
        };
        return { status: 202, body: analysis };
      }
    });
    open(`/knowledge?tab=commits&commit=${SHA}`, rt);
    fireEvent.click(await screen.findByRole("tab", { name: "AI review" }));
    fireEvent.click(await screen.findByRole("button", { name: "Analyse with AI" }));

    expect(await screen.findByText("Changes the agent lifecycle.")).toBeInTheDocument();
    expect(screen.getByText("Callers may hang")).toBeInTheDocument();
    expect(screen.getByText("High")).toBeInTheDocument();
    // Evidence chips show what the model was given, and findings without evidence are reported as discarded.
    expect(screen.getAllByText("run() (modified)").length).toBeGreaterThan(0);
    expect(screen.getByText(/2 findings without verifiable evidence were discarded/)).toBeInTheDocument();
    expect(rt.calls.some((c) => c.method === "POST" && c.path.endsWith("/analysis"))).toBe(true);

    fireEvent.click(screen.getByRole("tab", { name: "Tests" }));
    expect(await screen.findByText("Cancel during a turn")).toBeInTheDocument();
  });

  it("answers repository questions with citations", async () => {
    let messages: unknown[] = [];
    const rt = runtime({
      "GET /projects/:id/knowledge/chat": () => messages,
      "POST /projects/:id/knowledge/chat": ({ body }) => {
        messages = [
          { id: 1, project_id: "p1", role: "user", content: (body as { question: string }).question, evidence: [], status: "completed", provider: null, model: null, error: null, created_at: "2026-10-30T10:00:00Z" },
          {
            id: 2,
            project_id: "p1",
            role: "assistant",
            content: "Worktrees are created by the workspace manager [e1].",
            evidence: [{ id: "e1", kind: "symbol", label: "prepare_workspace()", file: "crates/symphony-workspaces/src/lib.rs", line: 420, node_id: "fn_prepare", sha: null }],
            status: "completed",
            provider: "codex",
            model: null,
            error: null,
            created_at: "2026-10-30T10:00:05Z"
          }
        ];
        return { status: 202, body: messages };
      }
    });
    open("/knowledge?tab=insights", rt);
    const input = await screen.findByRole("textbox", { name: "Ask about this repository" });
    fireEvent.change(input, { target: { value: "Where are worktrees created?" } });
    fireEvent.click(screen.getByRole("button", { name: "Ask" }));

    expect(await screen.findByText(/Worktrees are created by the workspace manager/)).toBeInTheDocument();
    expect(await screen.findByText("prepare_workspace()")).toBeInTheDocument();
    expect(rt.calls.find((c) => c.method === "POST")?.body).toEqual({ question: "Where are worktrees created?" });
  });

  it("finds precise references on request and says why when the navigator is unavailable", async () => {
    let available = true;
    const rt = runtime({
      "GET /projects/:id/knowledge/node": () => ({
        node: node({ id: "fn_run", label: "run()", kind: "function", file: "crates/core/src/lib.rs", line: 10, degree: 9 }),
        dependencies: [],
        references: [],
        members: [],
        dependencies_total: 0,
        references_total: 0,
        recent_commits: []
      }),
      "GET /projects/:id/knowledge/node/references": () =>
        available
          ? {
              node_id: "fn_run",
              available: true,
              reason: null,
              references: [{ file: "crates/server/src/lib.rs", name_path: "impl Server/serve", kind: "Method", start_line: 50, end_line: 60, reference_line: 55, snippet: "" }],
              total: 1,
              truncated: false,
              definition_start_line: 9,
              definition_end_line: 20
            }
          : { node_id: "fn_run", available: false, reason: "Serena is not installed.", references: [], total: 0, truncated: false, definition_start_line: null, definition_end_line: null }
    });
    const view = open("/knowledge?tab=graph&node=fn_run", rt);
    // Nothing is asked of the language server until the user asks.
    const find = await screen.findByRole("button", { name: "Find with the language server" });
    expect(rt.calls.some((c) => c.path.endsWith("/node/references"))).toBe(false);
    fireEvent.click(find);
    expect(await screen.findByText("impl Server/serve")).toBeInTheDocument();
    expect(screen.getByText("crates/server/src/lib.rs:55")).toBeInTheDocument();
    expect(screen.getByText("Defined on lines 9–20")).toBeInTheDocument();
    view.unmount();

    available = false;
    open("/knowledge?tab=graph&node=fn_run", rt);
    fireEvent.click(await screen.findByRole("button", { name: "Find with the language server" }));
    expect(await screen.findByText("Not available: Serena is not installed.")).toBeInTheDocument();
  });

  it("without an index the graph asks for one instead of drawing anything", async () => {
    const rt = runtime({ "GET /projects/:id/knowledge": () => overview({ index: null, top_modules: [], hubs: [], symbol_count: 0 }) });
    open("/knowledge?tab=graph", rt);
    expect(await screen.findByText("Index the repository to explore its graph.")).toBeInTheDocument();
    expect(canvas.last).toBeNull();
  });

  it("disables Sync when Graphify is not installed", async () => {
    const missing = engines.map((e) => (e.id === "graphify" ? { ...e, available: false, version: null } : e));
    open("/knowledge", runtime({ "GET /projects/:id/knowledge": () => overview({ engines: missing }) }));
    const sync = await screen.findByRole("button", { name: "Sync" });
    await waitFor(() => expect(sync).toBeDisabled());
  });

  it("switching repository drops the selected commit instead of asking the new one for it", async () => {
    const rt = runtime({ "GET /projects": () => [project({ id: "p1", name: "Demo repo" }), project({ id: "p2", name: "Other repo" })] });
    open(`/knowledge?tab=commits&commit=${SHA}`, rt);
    await waitFor(() => expect(rt.calls.some((c) => c.path.includes("/projects/p1/") && c.path.endsWith(`/commits/${SHA}`))).toBe(true));
    act(() => useUiStore.setState({ activeProjectId: "p2" }));
    expect(await screen.findByText("Select a commit to see what it changed and what could break.")).toBeInTheDocument();
    expect(rt.calls.some((c) => c.path.includes("/projects/p2/") && c.path.includes(SHA))).toBe(false);
  });

  it("a commit chosen on the strip from another tab opens its panel on the Commits tab", async () => {
    open("/knowledge");
    const strip = await screen.findByRole("region", { name: "Commits across all worktrees" });
    fireEvent.click(await within(strip).findByText("Fix agent lifecycle"));
    expect(await screen.findByRole("tab", { name: "Commits", selected: true })).toBeInTheDocument();
  });

  it("asks for a repository when none is registered", async () => {
    open("/knowledge", runtime({ "GET /projects": () => [] }));
    expect(await screen.findByText("Add a repository first")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add repository" })).toBeInTheDocument();
  });
});
