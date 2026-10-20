import { fireEvent, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import WorktreesPage from "./WorktreesPage.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { project, snapshot, worktree } from "../../test/fixtures.js";

function runtime() {
  return createMockRuntime({
    "GET /worktrees": () => [
      worktree({ task_id: "t1", issue_identifier: "SYM-1", issue_title: "Dirty work", dirty: true, workspace_path: "/ws/SYM-1" }),
      worktree({ task_id: "t2", issue_identifier: "SYM-2", issue_title: "Clean work", dirty: false, workspace_path: "/ws/SYM-2" })
    ],
    "GET /projects": () => [project({ id: "p1" })],
    "GET /state": () => snapshot(),
    "GET /tasks/:id/diff": () => ({
      task_id: "t1",
      workspace_path: "/ws/SYM-1",
      branch_name: "symphony/sym-1",
      exists: true,
      changed_files: [{ path: "x", status: "modified", additions: 1, deletions: 1 }],
      diff: "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old line\n+new line\n",
      truncated: false
    })
  });
}

describe("WorktreesPage", () => {
  it("lists worktrees without any delete action and hides Explorer outside the desktop host", async () => {
    renderWithProviders(<WorktreesPage />, { runtime: runtime(), route: "/worktrees", path: "/worktrees" });
    expect((await screen.findAllByText("SYM-1")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("SYM-2").length).toBeGreaterThan(0);
    expect(screen.queryByRole("button", { name: /delete|remove|clean ?up/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /Open in Explorer/ })).toBeNull();
  });

  it("opens the diff of a worktree", async () => {
    const rt = runtime();
    renderWithProviders(<WorktreesPage />, { runtime: rt, route: "/worktrees", path: "/worktrees" });
    fireEvent.click((await screen.findAllByRole("button", { name: /View diff/ }))[0]!);
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText(/new line/)).toBeInTheDocument();
    expect(rt.calls.some((c) => c.path.endsWith("/diff"))).toBe(true);
  });
});
