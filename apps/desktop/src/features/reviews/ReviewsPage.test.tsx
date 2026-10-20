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
    "POST /prs/:id/approve-merge": ({ path }) => {
      const id = decodeURIComponent(path.split("/")[2]!);
      const approved = { ...store.get(id)!, human_approved: true };
      store.set(id, approved);
      return approved;
    }
  });
}

const reviews = [
  pullRequest({ id: "pr-approved", pr_number: 1, title: "Approved docs", human_approved: true, updated_at: "2026-10-18T10:00:00Z" }),
  pullRequest({
    id: "pr-blocked",
    pr_number: 2,
    title: "Blocked refactor",
    merge_readiness: { score: 2, label: "Needs work", evidence: [], blockers: ["Validation failed"] },
    updated_at: "2026-10-20T09:00:00Z"
  }),
  pullRequest({
    id: "pr-ready",
    pr_number: 3,
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

    const confirm = within(dialog).getByRole("button", { name: "Record approval" });
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
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
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
    renderWithProviders(<ReviewsPage />, { runtime: runtimeWith([...reviews, web]), route: "/reviews/pr-ready", path: "/reviews/:reviewId" });
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
