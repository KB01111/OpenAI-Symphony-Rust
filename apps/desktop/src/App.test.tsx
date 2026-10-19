import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  canTransitionTaskStatus,
  getTranslations,
  type ApprovalRequest,
  type PullRequestReview
} from "@symphony/shared-types";
import {
  AiApprovalBanner,
  AiEmptyView,
  AiEventStreamCard,
  LogModeSwitch,
  MergeReadinessCard,
  PromptComposerBox,
  SymphonyDesignProvider
} from "@symphony/ui";
import { App } from "./App.js";

describe("Symphony Desktop 1.0 Frontend Suite", () => {
  it("validates Symphony task state machine transitions accurately", () => {
    expect(canTransitionTaskStatus("backlog", "ready")).toBe(true);
    expect(canTransitionTaskStatus("ready", "running")).toBe(true);
    expect(canTransitionTaskStatus("running", "human_review")).toBe(true);
    expect(canTransitionTaskStatus("backlog", "completed")).toBe(false);
  });

  it("provides complete English and Swedish localization dictionaries", () => {
    const en = getTranslations("en");
    const sv = getTranslations("sv");
    expect(en.nav.dashboard).toBe("Dashboard");
    expect(sv.nav.dashboard).toBe("Översikt");
    expect(sv.nav.taskBoard).toBe("Uppgiftstavla");
  });

  it("renders OpenAI Apps SDK UI approval banner and dispatches approval decisions", () => {
    const onDecide = vi.fn();
    const approval: ApprovalRequest = {
      id: "app-1",
      task_id: "task-1",
      issue_identifier: "SYM-42",
      session_id: "thread1-turn1",
      provider: "codex",
      request_id: "req-1",
      kind: "commandExecution",
      command: "cargo test --workspace",
      cwd: "C:\\workspaces\\SYM-42",
      file_paths: ["src/lib.rs"],
      reason: "Run workspace test suite",
      status: "pending",
      created_at: new Date().toISOString(),
      resolved_at: null
    };

    render(
      <SymphonyDesignProvider themeMode="dark">
        <AiApprovalBanner approval={approval} onDecide={onDecide} />
      </SymphonyDesignProvider>
    );

    expect(screen.getByText(/SYM-42/i)).toBeInTheDocument();
    expect(screen.getByText("--workspace")).toBeInTheDocument();

    fireEvent.click(screen.getByText("Approve Once"));
    expect(onDecide).toHaveBeenCalledWith("app-1", "approve");
  });

  it("renders structured 1-to-5 Merge Readiness report and evaluated review comments", () => {
    const onApprove = vi.fn();
    const review: PullRequestReview = {
      id: "pr-1",
      task_id: "task-1",
      project_id: "proj-1",
      pr_number: 101,
      title: "Implement Symphony Rust Scheduler",
      description: "Full SPEC.md compliant orchestrator",
      url: "https://github.com/KB01111/OpenAI-Symphony-Rust/pull/1",
      head_branch: "symphony/sym-101",
      base_branch: "main",
      changed_files: ["crates/symphony-orchestrator/src/lib.rs"],
      diff_preview: "+ pub struct SymphonyOrchestrator",
      ci_status: "passing",
      mergeable: true,
      merge_readiness: {
        score: 5,
        ci_passing: true,
        validation_passing: true,
        no_merge_conflicts: true,
        security_clean: true,
        reviewed_agent_comments: 1,
        rationale: ["All 17 Rust workspace tests passed"],
        warnings: []
      },
      comment_evaluations: [
        {
          author: "copilot-pull-request-reviewer[bot]",
          is_agent: true,
          body: "Verify workspace canonicalization check",
          file_path: "crates/symphony-workspaces/src/lib.rs",
          line: 98,
          verified_against_diff: true,
          evaluation_summary: "Verified against diff: canonical path guard is enforced."
        }
      ],
      human_approved: false,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString()
    };

    render(
      <SymphonyDesignProvider themeMode="dark">
        <MergeReadinessCard review={review} onApproveMerge={onApprove} />
      </SymphonyDesignProvider>
    );

    expect(screen.getByText(/Readiness 5 \/ 5/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Verified in Diff/i)
    ).toBeInTheDocument();

    fireEvent.click(screen.getByText("Approve & Complete PR"));
    expect(onApprove).toHaveBeenCalledWith("pr-1");
  });

  it("renders the full Symphony Desktop shell and navigates across sections", async () => {
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false } }
    });

    render(
      <QueryClientProvider client={qc}>
        <App />
      </QueryClientProvider>
    );

    expect(screen.getByText("Symphony Desktop")).toBeInTheDocument();

    // Switch language to Swedish (SV)
    fireEvent.click(screen.getByText("SV"));
    await waitFor(() => {
      expect(screen.getByText("Uppgiftstavla")).toBeInTheDocument();
    });

    // Switch back to English (EN)
    fireEvent.click(screen.getByText("EN"));
    await waitFor(() => {
      expect(screen.getByText("Task Board")).toBeInTheDocument();
    });

    // Navigate to Task Board
    fireEvent.click(screen.getByText("Task Board"));
    expect(screen.getByText("Backlog")).toBeInTheDocument();
    expect(screen.getByText("Human Review")).toBeInTheDocument();

    // Navigate to Settings and verify theme controls
    fireEvent.click(screen.getByText("Settings"));
    await waitFor(() => {
      expect(screen.getByText(/Appearance, Localization/i)).toBeInTheDocument();
    });

    // Open New Task modal form
    fireEvent.click(screen.getAllByText("New Development Task")[0]!);
    expect(screen.getAllByText("New Development Task").length).toBeGreaterThanOrEqual(2);
  });

  it("synchronizes theme mode across Fluent UI v9 and OpenAI Apps SDK UI document attributes", () => {
    const { rerender } = render(
      <SymphonyDesignProvider themeMode="dark">
        <div>Theme surface</div>
      </SymphonyDesignProvider>
    );
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

    rerender(
      <SymphonyDesignProvider themeMode="light">
        <div>Theme surface</div>
      </SymphonyDesignProvider>
    );
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("renders Apps SDK UI streaming event cards, log mode switcher, empty state, and prompt composer", () => {
    const onModeChange = vi.fn();
    const onSubmit = vi.fn();

    render(
      <SymphonyDesignProvider themeMode="dark">
        <div>
          <LogModeSwitch mode="concise" onChange={onModeChange} />
          <AiEventStreamCard
            detailed={true}
            event={{
              id: "ev-1",
              task_id: "task-1",
              issue_identifier: "SYM-77",
              session_id: "t1-u1",
              provider: "codex",
              event_type: "turn/completed",
              timestamp: new Date().toISOString(),
              turn_number: 1,
              summary: "Implemented **Git worktree** isolation.",
              payload: { files_changed: 3 },
              usage: {
                input_tokens: 1200,
                output_tokens: 450,
                total_tokens: 1650
              }
            }}
          />
          <AiEmptyView
            title="No Running Sessions"
            description="Select a ready task to start Codex App Server."
            actionLabel="Dispatch Now"
            onAction={() => {}}
          />
          <PromptComposerBox
            value="Add unit tests for workspace path safety"
            onChange={() => {}}
            onSubmit={onSubmit}
            submitLabel="Create Task"
          />
        </div>
      </SymphonyDesignProvider>
    );

    expect(screen.getByText(/Git worktree/i)).toBeInTheDocument();
    expect(screen.getByText("No Running Sessions")).toBeInTheDocument();

    fireEvent.click(screen.getByText("Create Task"));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});
