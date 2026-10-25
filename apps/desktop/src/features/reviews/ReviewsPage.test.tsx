import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { PullRequestReview } from "@symphony/shared-types";
import ReviewsPage from "./ReviewsPage.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { project, pullRequest } from "../../test/fixtures.js";

function runtimeWith(reviews: PullRequestReview[]) {
  const store = new Map(reviews.map((r) => [r.id, r]));
  return createMockRuntime({
    "GET /projects": () => [project()],
    "GET /prs": () => [...store.values()],
    "GET /prs/:id": ({ path }) => {
      const review = store.get(decodeURIComponent(path.split("/")[2]!));
      return review ?? { status: 404, body: { error: { code: "not_found", message: "not_found: pull request" } } };
    },
    "POST /prs/:id/create-pull-request": ({ path }) => {
      const id = decodeURIComponent(path.split("/")[2]!);
      const current = store.get(id)!;
      const opened = current.pr_number > 0
        ? { ...current, reviewed_commit: "def5678", unpushed_changes: false }
        : { ...current, pr_number: 42, url: "https://github.com/acme/widgets/pull/42", ci_status: "unknown", reviewed_commit: "abc1234" };
      store.set(id, opened);
      return opened;
    },
    "POST /prs/:id/refresh": ({ path }) => {
      const id = decodeURIComponent(path.split("/")[2]!);
      const refreshed = { ...store.get(id)!, ci_status: "passed", mergeable: true, has_conflicts: false };
      store.set(id, refreshed);
      return refreshed;
    },
    "POST /prs/:id/approve-merge": ({ path }) => {
      const id = decodeURIComponent(path.split("/")[2]!);
      const approved = { ...store.get(id)!, human_approved: true };
      store.set(id, approved);
      return approved;
    }
  });
}

const reviews = [
  pullRequest({ id: "pr-approved", pr_number: 1, url: "https://github.com/acme/widgets/pull/1", title: "Approved docs", human_approved: true, updated_at: "2026-10-18T10:00:00Z" }),
  pullRequest({
    id: "pr-blocked",
    pr_number: 2,
    url: "https://github.com/acme/widgets/pull/2",
    title: "Blocked refactor",
    merge_readiness: { score: 2, label: "Needs work", evidence: [], blockers: ["Validation failed"] },
    updated_at: "2026-10-20T09:00:00Z"
  }),
  pullRequest({
    id: "pr-ready",
    pr_number: 3,
    url: "https://github.com/acme/widgets/pull/3",
    title: "Ready feature",
    merge_readiness: { score: 5, label: "Ready", evidence: ["All checks passed"], blockers: [] },
    updated_at: "2026-10-19T09:00:00Z"
  })
];

function listTitles() {
  const list = screen.getByRole("list", { name: "Pull request reviews" });
  return within(list)
    .getAllByRole("button")
    .map((b) => b.textContent ?? "");
}

describe("ReviewsPage", () => {
  it("does not invent a pull request number or a CI result for a local worktree review", async () => {
    const local = pullRequest({ id: "pr-local", pr_number: 0, ci_status: "none", title: "Local branch only" });
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([local]), route: "/reviews", path: "/reviews" });
    await screen.findByText("Local branch only");
    const item = listTitles()[0]!;
    expect(item).toContain("Local review");
    expect(item).toContain("No remote CI");
    expect(item).not.toContain("#0");
    expect(item).not.toMatch(/CI: (none|passed)/);
  });

  it("creates the pull request only after an explicit confirmation, exactly once", async () => {
    const local = pullRequest({ id: "pr-local", pr_number: 0, ci_status: "none", title: "Local branch only", branch_name: "symphony/sym-9", base_branch: "main" });
    const runtime = runtimeWith([local]);
    renderWithProviders(<ReviewsPage />, { runtime, route: "/reviews/pr-local", path: "/reviews/:reviewId" });
    fireEvent.click(await screen.findByRole("button", { name: "Create pull request" }));

    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/commits the worktree's changes to branch symphony\/sym-9/)).toBeInTheDocument();
    expect(within(dialog).getByText(/Nothing is merged/)).toBeInTheDocument();
    expect(runtime.calls.filter((c) => c.method === "POST")).toHaveLength(0);

    const confirm = await within(dialog).findByRole("button", { name: "Create pull request" });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(runtime.calls.filter((c) => c.method === "POST")).toEqual([
      { method: "POST", path: "/prs/pr-local/create-pull-request", body: undefined }
    ]);
    // The review now links the real pull request, and the action is gone.
    expect(await screen.findByRole("link", { name: /github.com\/acme\/widgets\/pull\/42/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Create pull request" })).not.toBeInTheDocument();
  });

  it("refreshes a published pull request from GitHub on request", async () => {
    const published = pullRequest({ id: "pr-live", pr_number: 9, ci_status: "unknown", url: "https://github.com/acme/widgets/pull/9", title: "Published", reviewed_commit: "abc1234" });
    const runtime = runtimeWith([published]);
    renderWithProviders(<ReviewsPage />, { runtime, route: "/reviews/pr-live", path: "/reviews/:reviewId" });
    fireEvent.click(await screen.findByRole("button", { name: "Refresh status" }));
    await waitFor(() =>
      expect(runtime.calls.filter((c) => c.method === "POST")).toEqual([{ method: "POST", path: "/prs/pr-live/refresh", body: undefined }])
    );
    // A local review has nothing to refresh.
    expect(screen.queryByRole("button", { name: "Create pull request" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Push changes/ })).not.toBeInTheDocument();
  });

  it("does not offer a push for a review published by an older release (no recorded commit)", async () => {
    const legacy = pullRequest({ id: "pr-old", pr_number: 9, url: "https://github.com/acme/widgets/pull/9", title: "Older", reviewed_commit: null, unpushed_changes: false });
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([legacy]), route: "/reviews/pr-old", path: "/reviews/:reviewId" });
    expect(await screen.findByRole("button", { name: "Refresh status" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Push changes/ })).not.toBeInTheDocument();
  });

  it("offers to push a continued run's changes to the pull request that is already open", async () => {
    // The task ran again after #9 was opened: the review links #9, but its work was never pushed.
    const continued = pullRequest({ id: "pr-cont", pr_number: 9, url: "https://github.com/acme/widgets/pull/9", title: "Continued", branch_name: "symphony/sym-9", unpushed_changes: true });
    const runtime = runtimeWith([continued]);
    renderWithProviders(<ReviewsPage />, { runtime, route: "/reviews/pr-cont", path: "/reviews/:reviewId" });
    fireEvent.click(await screen.findByRole("button", { name: "Push changes to #9" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/updates pull request #9/)).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Push changes" }));
    await waitFor(() =>
      expect(runtime.calls.filter((c) => c.method === "POST")).toEqual([
        { method: "POST", path: "/prs/pr-cont/create-pull-request", body: undefined }
      ])
    );
    await waitFor(() => expect(screen.queryByRole("button", { name: /Push changes/ })).not.toBeInTheDocument());
  });

  it("blocks creating a pull request while the branch conflicts or has security warnings", async () => {
    const conflicted = pullRequest({ id: "pr-conflict", pr_number: 0, ci_status: "none", has_conflicts: true, mergeable: false, title: "Conflicted" });
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([conflicted]), route: "/reviews/pr-conflict", path: "/reviews/:reviewId" });
    const button = await screen.findByRole("button", { name: "Create pull request" });
    expect(button).toBeDisabled();
    expect(screen.getAllByText(/Resolve the merge conflicts/).length).toBeGreaterThan(0);
  });

  it("blocks creating a pull request while the change has security warnings", async () => {
    const flagged = pullRequest({
      id: "pr-flagged",
      pr_number: 0,
      ci_status: "none",
      security_warnings: ["Possible GitHub token added by the change."],
      title: "Flagged"
    });
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([flagged]), route: "/reviews/pr-flagged", path: "/reviews/:reviewId" });
    const button = await screen.findByRole("button", { name: "Create pull request" });
    expect(button).toBeDisabled();
    expect(screen.queryByText(/Resolve the merge conflicts/)).not.toBeInTheDocument();
    expect(screen.getAllByText(/security warning/i).length).toBeGreaterThan(0);
  });

  it("surfaces a refused pull request instead of pretending it worked", async () => {
    const local = pullRequest({ id: "pr-local", pr_number: 0, ci_status: "none", title: "Local branch only" });
    const runtime = createMockRuntime({
      "GET /projects": () => [project()],
      "GET /prs": () => [local],
      "GET /prs/:id": () => local,
      "POST /prs/:id/create-pull-request": () => ({
        status: 400,
        body: { error: { code: "invalid_config", message: "invalid_config: no GitHub token is configured" } }
      })
    });
    renderWithProviders(<ReviewsPage />, { runtime, route: "/reviews/pr-local", path: "/reviews/:reviewId" });
    fireEvent.click(await screen.findByRole("button", { name: "Create pull request" }));
    const dialog = await screen.findByRole("alertdialog");
    fireEvent.click(await within(dialog).findByRole("button", { name: "Create pull request" }));
    expect(await screen.findByText(/no GitHub token is configured/)).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Create pull request" })).toBeInTheDocument();
  });

  it("lists reviews newest first and filters them", async () => {
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith(reviews), route: "/reviews", path: "/reviews" });
    await screen.findByText("Blocked refactor");
    expect(listTitles().map((t) => t.match(/#\d/)?.[0])).toEqual(["#2", "#3", "#1"]);

    fireEvent.click(screen.getByRole("checkbox", { name: "Has blockers" }));
    await waitFor(() => expect(listTitles()).toHaveLength(1));
    expect(listTitles()[0]).toContain("Blocked refactor");
    fireEvent.click(screen.getByRole("checkbox", { name: "Has blockers" }));

    fireEvent.click(screen.getByRole("combobox", { name: "Approval" }));
    fireEvent.click(await screen.findByRole("option", { name: "Approved" }));
    await waitFor(() => expect(listTitles()).toHaveLength(1));
    expect(listTitles()[0]).toContain("Approved docs");

    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }));
    fireEvent.change(screen.getByRole("searchbox", { name: "Search reviews" }), { target: { value: "ready" } });
    await waitFor(() => expect(listTitles()).toHaveLength(1));
    expect(listTitles()[0]).toContain("Ready feature");
  });

  it("filters by minimum readiness score", async () => {
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith(reviews), route: "/reviews", path: "/reviews" });
    await screen.findByText("Blocked refactor");
    fireEvent.click(screen.getByRole("combobox", { name: "Readiness score" }));
    fireEvent.click(await screen.findByRole("option", { name: "Score ≥ 5" }));
    await waitFor(() => expect(listTitles()).toHaveLength(1));
    expect(listTitles()[0]).toContain("Ready feature");
  });

  it("explains where reviews come from when there are none", async () => {
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([]), route: "/reviews", path: "/reviews" });
    expect(await screen.findByText("No pull request reviews yet")).toBeInTheDocument();
    expect(screen.getByText(/produced automatically when an agent run completes/)).toBeInTheDocument();
  });

  it("requires explicit confirmation before recording approval, and approves exactly once", async () => {
    const runtime = runtimeWith(reviews);
    renderWithProviders(<ReviewsPage />, { runtime, route: "/reviews/pr-ready", path: "/reviews/:reviewId" });
    const approve = await screen.findByRole("button", { name: "Record human approval" });
    fireEvent.click(approve);

    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/does NOT merge/)).toBeInTheDocument();
    expect(within(dialog).getByText(/marks the task as completed/)).toBeInTheDocument();
    expect(runtime.calls.filter((c) => c.method === "POST")).toHaveLength(0);

    const confirm = await within(dialog).findByRole("button", { name: "Record approval" });
    fireEvent.click(confirm);
    fireEvent.click(confirm);

    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    const posts = runtime.calls.filter((c) => c.method === "POST");
    expect(posts).toEqual([{ method: "POST", path: "/prs/pr-ready/approve-merge", body: undefined }]);
    await waitFor(() => expect(screen.queryByRole("button", { name: "Record human approval" })).not.toBeInTheDocument());
  });

  it("does not record approval when the dialog is cancelled", async () => {
    const runtime = runtimeWith(reviews);
    renderWithProviders(<ReviewsPage />, { runtime, route: "/reviews/pr-ready", path: "/reviews/:reviewId" });
    fireEvent.click(await screen.findByRole("button", { name: "Record human approval" }));
    const dialog = await screen.findByRole("alertdialog");
    fireEvent.click(await within(dialog).findByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(runtime.calls.filter((c) => c.method === "POST")).toHaveLength(0);
  });

  it("shows no approve button for already approved pull requests", async () => {
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith(reviews), route: "/reviews/pr-approved", path: "/reviews/:reviewId" });
    expect(await screen.findByRole("heading", { name: /Approved docs/ })).toBeInTheDocument();
    expect(screen.getByText("Human approved")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Record human approval" })).not.toBeInTheDocument();
  });

  it("renders local:// urls as text and web urls as safe links", async () => {
    const web = pullRequest({ id: "pr-web", url: "https://github.com/acme/repo/pull/7", title: "Web PR" });
    const local = pullRequest({ id: "pr-local-url", pr_number: 0, ci_status: "none", url: "local://worktree/symphony/sym-1", title: "Local only" });
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([local, web]), route: "/reviews/pr-local-url", path: "/reviews/:reviewId" });
    expect(await screen.findByText("local://worktree/symphony/sym-1")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /local:\/\// })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Web PR/ }));
    const link = await screen.findByRole("link", { name: /github\.com\/acme\/repo\/pull\/7/ });
    expect(link).toHaveAttribute("rel", "noreferrer");
  });

  it("renders AI review comments next to the matching file diff", async () => {
    const withComments = pullRequest({
      id: "pr-comments",
      review_comments: [
        {
          author: "codex-reviewer",
          is_agent: true,
          body: "Consider documenting the task note.",
          file_path: "SYMPHONY_TASK_NOTE.md",
          verified_against_diff: true,
          evaluation_summary: "Valid suggestion"
        },
        {
          author: "maintainer",
          is_agent: false,
          body: "Looks fine overall.",
          file_path: null,
          verified_against_diff: false,
          evaluation_summary: ""
        }
      ]
    });
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([withComments]), route: "/reviews/pr-comments", path: "/reviews/:reviewId" });
    fireEvent.click(await screen.findByRole("tab", { name: /AI review comments/ }));

    const comment = await screen.findByRole("article", { name: "codex-reviewer · SYMPHONY_TASK_NOTE.md" });
    expect(within(comment).getByText("Agent")).toBeInTheDocument();
    expect(within(comment).getByText("Verified against diff")).toBeInTheDocument();
    expect(within(comment).getByText("Valid suggestion")).toBeInTheDocument();
    const diff = within(comment).getByRole("figure", { name: "Diff for SYMPHONY_TASK_NOTE.md" });
    expect(within(diff).getByText("+# Task")).toBeInTheDocument();

    const general = screen.getByRole("article", { name: "maintainer" });
    expect(within(general).queryByRole("figure")).not.toBeInTheDocument();
    expect(within(general).getByText("Human")).toBeInTheDocument();
  });

  it("selecting a changed file shows the diff filtered to that file", async () => {
    const twoFiles = pullRequest({
      id: "pr-files",
      changed_files: [
        { path: "src/a.rs", status: "modified", additions: 1, deletions: 1 },
        { path: "README.md", status: "added", additions: 1, deletions: 0 }
      ],
      diff_preview: [
        "diff --git a/src/a.rs b/src/a.rs",
        "+++ b/src/a.rs",
        "@@ -1 +1 @@",
        "-old",
        "+new",
        "diff --git a/README.md b/README.md",
        "+++ b/README.md",
        "@@ -0,0 +1 @@",
        "+# Readme"
      ].join("\n")
    });
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([twoFiles]), route: "/reviews/pr-files", path: "/reviews/:reviewId" });
    fireEvent.click(await screen.findByRole("button", { name: "Show diff for README.md" }));
    expect(await screen.findByText("Showing README.md")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Diff", selected: true })).toBeInTheDocument();
    expect(screen.queryByText("src/a.rs")).not.toBeInTheDocument();
  });

  it("shows a not-found state for unknown review ids", async () => {
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith(reviews), route: "/reviews/nope", path: "/reviews/:reviewId" });
    expect(await screen.findByText("Pull request review not found")).toBeInTheDocument();
  });
});
