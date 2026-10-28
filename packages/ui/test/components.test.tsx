import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AgentEvent, ApprovalRequest, PullRequestReview } from "@symphony/shared-types";
import {
  AiApprovalBanner,
  AiEventStreamCard,
  DataTable,
  MergeReadinessCard,
  CodeBlock,
  SymphonyDesignProvider,
  TaskStatusBadge,
  formatAgentEvent,
  readinessEvidence,
  type ColumnDef
} from "../src/index.js";
import { setMedia } from "./setup.js";

const baseEvent: AgentEvent = {
  id: "e1",
  task_id: "t1",
  issue_identifier: "SYM-1",
  session_id: "s1",
  provider: "simulated",
  event_type: "item/agentMessage",
  phase: "streaming_turn",
  message: "Planning **changes**",
  payload: { item: { type: "agentMessage", text: "Planning **changes**" } },
  timestamp: "2026-01-01T10:00:00Z"
};

describe("SymphonyDesignProvider", () => {
  it("applies explicit and system themes to <html> and follows OS changes", () => {
    const { rerender } = render(
      <SymphonyDesignProvider themeMode="light" density="comfortable" locale="sv">
        <span>child</span>
      </SymphonyDesignProvider>
    );
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.dataset.density).toBe("comfortable");
    expect(document.documentElement.lang).toBe("sv");

    rerender(
      <SymphonyDesignProvider themeMode="system">
        <span>child</span>
      </SymphonyDesignProvider>
    );
    expect(document.documentElement.dataset.theme).toBe("dark");
    act(() => setMedia("(prefers-color-scheme: dark)", false));
    expect(document.documentElement.dataset.theme).toBe("light");
    act(() => setMedia("(prefers-color-scheme: dark)", true));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.dataset.density).toBe("compact");
  });

  it("localizes shared components", () => {
    render(
      <SymphonyDesignProvider themeMode="dark" locale="sv">
        <TaskStatusBadge status="human_review" />
      </SymphonyDesignProvider>
    );
    expect(screen.getByText("Mänsklig granskning")).toBeInTheDocument();
  });
});

describe("formatAgentEvent", () => {
  it("classifies provider item events", () => {
    expect(formatAgentEvent(baseEvent).kind).toBe("message");
    const cmd = formatAgentEvent({
      ...baseEvent,
      event_type: "item/commandExecution",
      payload: { item: { type: "commandExecution", command: "git status", exitCode: 2, aggregatedOutput: "x" } }
    });
    expect(cmd).toMatchObject({ kind: "command", command: "git status", exitCode: 2, tone: "danger" });
    const file = formatAgentEvent({
      ...baseEvent,
      event_type: "item/fileChange",
      payload: { item: { type: "fileChange", changes: [{ path: "a.md", kind: "add", diff: "+x" }] } }
    });
    expect(file.files).toEqual([{ path: "a.md", kind: "add", diff: "+x" }]);
    // Codex App Server notifications are stored as the raw JSON-RPC message.
    const codexFile = formatAgentEvent({
      ...baseEvent,
      event_type: "item/completed",
      payload: {
        method: "item/completed",
        params: { item: { type: "fileChange", changes: [{ path: "src/lib.rs", kind: { type: "update" }, diff: "-a\n+b" }] } }
      }
    });
    expect(codexFile).toMatchObject({ kind: "file_change", files: [{ path: "src/lib.rs", kind: "update", diff: "-a\n+b" }] });
    const codexCmd = formatAgentEvent({
      ...baseEvent,
      event_type: "item/started",
      payload: { method: "item/started", params: { item: { type: "commandExecution", command: "cargo test", exitCode: 0 } } }
    });
    expect(codexCmd).toMatchObject({ kind: "command", command: "cargo test", exitCode: 0 });
    expect(formatAgentEvent({ ...baseEvent, event_type: "attempt/failed", payload: null }).kind).toBe("error");
    expect(formatAgentEvent({ ...baseEvent, event_type: "approval/requested", payload: null }).tone).toBe("warning");
    expect(formatAgentEvent({ ...baseEvent, event_type: "attempt/succeeded", payload: null }).tone).toBe("success");
  });
});

describe("AiEventStreamCard", () => {
  it("shows raw payload only in technical mode", () => {
    const { rerender } = render(
      <SymphonyDesignProvider themeMode="dark">
        <AiEventStreamCard event={baseEvent} mode="concise" />
      </SymphonyDesignProvider>
    );
    expect(screen.queryByText("Raw payload")).not.toBeInTheDocument();
    rerender(
      <SymphonyDesignProvider themeMode="dark">
        <AiEventStreamCard event={baseEvent} mode="technical" />
      </SymphonyDesignProvider>
    );
    expect(screen.getByText("item/agentMessage")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Raw payload"));
    expect(screen.getByText("Hide payload")).toBeInTheDocument();
  });

  it("names every copy button, including the ones in code blocks and Markdown code", () => {
    const text = "Run this:\n\n```bash\ncargo test\n```";
    const withCode: AgentEvent = { ...baseEvent, message: text, payload: { item: { type: "agentMessage", text } } };
    render(
      <SymphonyDesignProvider themeMode="dark" locale="sv">
        <AiEventStreamCard event={withCode} mode="technical" />
        <CodeBlock language="json">{"{}"}</CodeBlock>
      </SymphonyDesignProvider>
    );
    for (const button of screen.getAllByRole("button")) expect(button).toHaveAccessibleName();
    expect(screen.getByRole("button", { name: "Kopiera händelsetext" })).toBeInTheDocument();
    // One in the Markdown code of the event, one in the standalone block.
    expect(screen.getAllByRole("button", { name: "Kopiera kod" })).toHaveLength(2);
  });
});

const approval: ApprovalRequest = {
  id: "a1",
  task_id: "t1",
  issue_identifier: "SYM-1",
  session_id: "s1",
  provider: "codex",
  kind: "command_execution",
  title: "Run tests",
  description: "Agent wants to run tests",
  command_or_diff: "cargo test",
  status: "pending",
  created_at: "2026-01-01T00:00:00Z",
  resolved_at: null,
  decision: null
};

describe("AiApprovalBanner", () => {
  it("reports decisions and locks while one is pending", () => {
    const onDecide = vi.fn();
    const { rerender } = render(
      <SymphonyDesignProvider themeMode="dark">
        <AiApprovalBanner approval={approval} onDecide={onDecide} />
      </SymphonyDesignProvider>
    );
    fireEvent.click(screen.getByRole("button", { name: "Approve for session" }));
    expect(onDecide).toHaveBeenCalledWith(approval, "accept_for_session");
    rerender(
      <SymphonyDesignProvider themeMode="dark">
        <AiApprovalBanner approval={approval} onDecide={onDecide} pendingDecision="accept_for_session" />
      </SymphonyDesignProvider>
    );
    for (const name of ["Approve once", "Decline", "Abort turn"]) {
      expect(screen.getByRole("button", { name })).toBeDisabled();
    }
    expect(screen.getByRole("status")).toHaveTextContent("Submitting decision");
  });
});

const review: PullRequestReview = {
  id: "pr-1",
  task_id: "t1",
  project_id: "p1",
  pr_number: 7,
  title: "SYM-1: change",
  description: "",
  url: "local://x",
  branch_name: "symphony/sym-1",
  base_branch: "main",
  state: "open",
  ci_status: "passed",
  mergeable: true,
  has_conflicts: false,
  changed_files: [],
  diff_preview: "",
  validation_results: [],
  review_comments: [],
  security_warnings: ["leaked key"],
  merge_readiness: { score: 2, label: "Not ready", evidence: ["Clean merge"], blockers: ["Security warning: leaked key"] },
  reviewed_commit: null,
  unpushed_changes: false,
  human_approved: false,
  updated_at: "2026-01-01T00:00:00Z"
};

describe("MergeReadinessCard", () => {
  it("derives evidence and requires an explicit approval action", () => {
    expect(readinessEvidence(review)).toEqual({
      ci: "pass",
      validation: "pending",
      conflicts: "pass",
      security: "fail",
      reviewedComments: 0
    });
    // An unchecked merge (not mergeable, no conflicts) is pending, never "clean".
    expect(readinessEvidence({ ...review, mergeable: false }).conflicts).toBe("pending");
    expect(readinessEvidence({ ...review, has_conflicts: true, mergeable: false }).conflicts).toBe("fail");
    const onApprove = vi.fn();
    render(
      <SymphonyDesignProvider themeMode="dark">
        <MergeReadinessCard review={review} onApprove={onApprove} />
      </SymphonyDesignProvider>
    );
    expect(screen.getByRole("meter")).toHaveAttribute("aria-valuenow", "2");
    expect(screen.getByText("Security warning: leaked key")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Record human approval/ }));
    expect(onApprove).toHaveBeenCalledWith(review);
  });
});

describe("localized runtime events", () => {
  const reloaded: AgentEvent = {
    ...baseEvent,
    id: "w1",
    task_id: null,
    issue_identifier: null,
    session_id: null,
    provider: null,
    phase: null,
    event_type: "workflow/reloaded",
    message: "Reloaded workflow from /repo/WORKFLOW.md",
    payload: { path: "/repo/WORKFLOW.md" }
  };

  it("renders workflow events from the UI locale instead of the runtime message", () => {
    render(
      <SymphonyDesignProvider themeMode="light" locale="sv">
        <AiEventStreamCard event={reloaded} />
      </SymphonyDesignProvider>
    );
    expect(screen.getByText("WORKFLOW.md lästes in på nytt från /repo/WORKFLOW.md")).toBeTruthy();
    expect(screen.queryByText(reloaded.message)).toBeNull();
  });
});


describe("DataTable", () => {
  interface Item {
    id: string;
    name: string;
    branch: string;
    tokens: number;
  }
  const items: Item[] = [
    { id: "b", name: "beta", branch: "feature-10", tokens: 30 },
    { id: "a", name: "alpha", branch: "feature-9", tokens: 100 },
    { id: "c", name: "gamma", branch: "feature-2", tokens: 5 }
  ];
  const columns: ColumnDef<Item, unknown>[] = [
    { id: "name", header: "Name", accessorFn: (i) => i.name, cell: ({ row }) => row.original.name },
    { id: "branch", header: "Branch", accessorFn: (i) => i.branch, cell: ({ row }) => row.original.branch },
    { id: "tokens", header: "Tokens", accessorFn: (i) => i.tokens, cell: ({ row }) => String(row.original.tokens) }
  ];
  const firstCells = () =>
    screen
      .getAllByRole("row")
      .slice(1)
      .map((r) => r.querySelector("td")?.textContent);

  it("sorts strings, alphanumeric values and numbers from the header and reports aria-sort", () => {
    render(<DataTable data={items} columns={columns} getRowId={(i) => i.id} ariaLabel="Items" />);
    expect(firstCells()).toEqual(["beta", "alpha", "gamma"]);

    fireEvent.click(screen.getByRole("button", { name: /Name/ }));
    expect(firstCells()).toEqual(["alpha", "beta", "gamma"]);
    expect(screen.getByRole("columnheader", { name: /Name/ })).toHaveAttribute("aria-sort", "ascending");
    fireEvent.click(screen.getByRole("button", { name: /Name/ }));
    expect(firstCells()).toEqual(["gamma", "beta", "alpha"]);
    expect(screen.getByRole("columnheader", { name: /Name/ })).toHaveAttribute("aria-sort", "descending");

    // Alphanumeric: feature-2 < feature-9 < feature-10 (not lexicographic).
    fireEvent.click(screen.getByRole("button", { name: /Branch/ }));
    expect(firstCells()).toEqual(["gamma", "alpha", "beta"]);

    // Numbers sort descending first, like TanStack Table v8.
    fireEvent.click(screen.getByRole("button", { name: /Tokens/ }));
    expect(firstCells()).toEqual(["alpha", "beta", "gamma"]);
  });

  it("activates rows by click and keyboard", () => {
    const onRowActivate = vi.fn();
    render(
      <DataTable data={items} columns={columns} getRowId={(i) => i.id} ariaLabel="Items" selectedId="a" onRowActivate={onRowActivate} />
    );
    const rows = screen.getAllByRole("row").slice(1);
    for (const row of rows) expect(row).toHaveAttribute("tabindex", "0");
    expect(rows[1]).toHaveAttribute("aria-selected", "true");
    fireEvent.click(rows[0]!);
    fireEvent.keyDown(rows[2]!, { key: "Enter" });
    expect(onRowActivate.mock.calls.map(([i]) => (i as Item).id)).toEqual(["b", "c"]);
  });
});
