import { describe, expect, it } from "vitest";
import { pullRequest } from "../../test/fixtures.js";
import { parseUnifiedDiff } from "../../components/DiffViewer.js";
import {
  DEFAULT_REVIEW_FILTERS,
  ciTone,
  diffFileForComment,
  diffForFile,
  filterReviews,
  hasActiveReviewFilters,
  isWebUrl,
  scoreTone
} from "./reviewHelpers.js";

const reviews = [
  pullRequest({ id: "a", title: "Old approved", human_approved: true, updated_at: "2026-10-01T00:00:00Z" }),
  pullRequest({
    id: "b",
    title: "Blocked change",
    branch_name: "symphony/sym-2",
    merge_readiness: { score: 2, label: "Blocked", evidence: [], blockers: ["CI failed"] },
    updated_at: "2026-10-03T00:00:00Z"
  }),
  pullRequest({
    id: "c",
    title: "Ready change",
    project_id: "p2",
    merge_readiness: { score: 5, label: "Ready", evidence: [], blockers: [] },
    updated_at: "2026-10-02T00:00:00Z"
  })
];

describe("filterReviews", () => {
  it("sorts by updated_at, newest first", () => {
    expect(filterReviews(reviews, DEFAULT_REVIEW_FILTERS).map((r) => r.id)).toEqual(["b", "c", "a"]);
  });

  it("filters by approval state", () => {
    expect(filterReviews(reviews, { ...DEFAULT_REVIEW_FILTERS, approval: "approved" }).map((r) => r.id)).toEqual(["a"]);
    expect(filterReviews(reviews, { ...DEFAULT_REVIEW_FILTERS, approval: "awaiting" }).map((r) => r.id)).toEqual(["b", "c"]);
  });

  it("filters by minimum readiness score and blockers", () => {
    expect(filterReviews(reviews, { ...DEFAULT_REVIEW_FILTERS, minScore: 4 }).map((r) => r.id)).toEqual(["c", "a"]);
    expect(filterReviews(reviews, { ...DEFAULT_REVIEW_FILTERS, hasBlockers: true }).map((r) => r.id)).toEqual(["b"]);
  });

  it("searches title, branch, number and project name", () => {
    expect(filterReviews(reviews, { ...DEFAULT_REVIEW_FILTERS, search: "sym-2" }).map((r) => r.id)).toEqual(["b"]);
    const names = new Map([["p2", "Billing service"]]);
    expect(filterReviews(reviews, { ...DEFAULT_REVIEW_FILTERS, search: "billing" }, names).map((r) => r.id)).toEqual(["c"]);
  });

  it("detects active filters", () => {
    expect(hasActiveReviewFilters(DEFAULT_REVIEW_FILTERS)).toBe(false);
    expect(hasActiveReviewFilters({ ...DEFAULT_REVIEW_FILTERS, hasBlockers: true })).toBe(true);
  });
});

describe("tones and urls", () => {
  it("maps scores and CI status to semantic tones", () => {
    expect([1, 2, 3, 4, 5].map(scoreTone)).toEqual(["danger", "danger", "warning", "success", "success"]);
    expect(ciTone("passed")).toBe("success");
    expect(ciTone("FAILURE")).toBe("danger");
    expect(ciTone("pending")).toBe("pending");
  });

  it("only treats http(s) urls as web links", () => {
    expect(isWebUrl("https://github.com/o/r/pull/1")).toBe(true);
    expect(isWebUrl("local://worktree/symphony/sym-1")).toBe(false);
    expect(isWebUrl("javascript:alert(1)")).toBe(false);
    expect(isWebUrl("not a url")).toBe(false);
  });
});

describe("diff slicing", () => {
  const diff = [
    "diff --git a/src/a.rs b/src/a.rs",
    "--- a/src/a.rs",
    "+++ b/src/a.rs",
    "@@ -1 +1 @@",
    "-old",
    "+new",
    "diff --git a/README.md b/README.md",
    "+++ b/README.md",
    "@@ -0,0 +1 @@",
    "+# Readme"
  ].join("\n");

  it("restricts a unified diff to one file", () => {
    const only = diffForFile(diff, "README.md");
    expect(only).toContain("+# Readme");
    expect(only).not.toContain("+new");
    expect(diffForFile(diff, "missing.txt")).toBe("");
  });

  it("matches review comments to parsed diff files", () => {
    const files = parseUnifiedDiff(diff);
    const comment = { author: "agent", is_agent: true, body: "", file_path: "./src/a.rs", verified_against_diff: true, evaluation_summary: "" };
    expect(diffFileForComment(comment, files)?.path).toBe("src/a.rs");
    expect(diffFileForComment({ ...comment, file_path: null }, files)).toBeUndefined();
  });
});
