/**
 * Pure helpers for the Pull Request Review Center (filtering, tone mapping, diff slicing).
 * Kept free of React so they can be unit tested in isolation.
 */
import type { PullRequestReview, ReviewCommentEvaluation } from "@symphony/shared-types";
import { parseUnifiedDiff, type DiffFile } from "../../components/DiffViewer.js";

export type ApprovalFilter = "all" | "awaiting" | "approved";

export interface ReviewFilters {
  search: string;
  approval: ApprovalFilter;
  /** Only show reviews whose readiness score is at least this value (1–5). */
  minScore: number | null;
  hasBlockers: boolean;
}

export const DEFAULT_REVIEW_FILTERS: ReviewFilters = { search: "", approval: "all", minScore: null, hasBlockers: false };

export function hasActiveReviewFilters(filters: ReviewFilters): boolean {
  return filters.search.trim() !== "" || filters.approval !== "all" || filters.minScore !== null || filters.hasBlockers;
}

function timestamp(iso: string): number {
  const value = Date.parse(iso);
  return Number.isNaN(value) ? 0 : value;
}

/** Filter by search text / approval state / minimum score / blockers, newest update first. */
export function filterReviews(
  reviews: ReadonlyArray<PullRequestReview>,
  filters: ReviewFilters,
  projectNames: ReadonlyMap<string, string> = new Map()
): PullRequestReview[] {
  const needle = filters.search.trim().toLowerCase();
  return reviews
    .filter((r) => {
      if (filters.approval === "approved" && !r.human_approved) return false;
      if (filters.approval === "awaiting" && r.human_approved) return false;
      if (filters.minScore !== null && r.merge_readiness.score < filters.minScore) return false;
      if (filters.hasBlockers && r.merge_readiness.blockers.length === 0) return false;
      if (!needle) return true;
      const haystack = [r.title, r.branch_name, r.base_branch, r.pr_number > 0 ? `#${r.pr_number}` : "", r.task_id, projectNames.get(r.project_id) ?? ""]
        .join("\n")
        .toLowerCase();
      return haystack.includes(needle);
    })
    .sort((a, b) => timestamp(b.updated_at) - timestamp(a.updated_at));
}

export type ScoreTone = "success" | "warning" | "danger";

/** Readiness score → semantic tone (4–5 good, 3 caution, 1–2 blocked). */
export function scoreTone(score: number): ScoreTone {
  return score >= 4 ? "success" : score === 3 ? "warning" : "danger";
}

export const SCORE_CHIP_CLASS: Record<ScoreTone, string> = {
  success: "bg-success-subtle text-success",
  warning: "bg-warning-subtle text-warning",
  danger: "bg-destructive-subtle text-destructive"
};

/** `pr_number` 0 means no pull request exists: the review covers a local worktree branch. */
export function hasPullRequest(review: { pr_number: number }): boolean {
  return review.pr_number > 0;
}

export type CiTone = "success" | "danger" | "pending";

export function ciTone(ciStatus: string): CiTone {
  const ci = ciStatus.trim().toLowerCase();
  if (ci === "success" || ci === "passed") return "success";
  if (ci === "failure" || ci === "failed" || ci === "error") return "danger";
  return "pending";
}

/** Only real web URLs are rendered as links; runtime-local `local://` references are plain text. */
export function isWebUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

function normalizePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/^(\.\/|[ab]\/)/, "");
}

/** Raw diff text restricted to the sections that touch `path` (empty when the file has no hunk). */
export function diffForFile(diff: string, path: string): string {
  const target = normalizePath(path);
  const sections: string[] = [];
  let current: string[] | null = null;
  for (const line of diff.split("\n")) {
    if (line.startsWith("diff --git ")) {
      if (current) sections.push(current.join("\n"));
      current = [line];
    } else if (current) {
      current.push(line);
    }
  }
  if (current) sections.push(current.join("\n"));
  return sections.filter((section) => parseUnifiedDiff(section).some((f) => normalizePath(f.path) === target)).join("\n");
}

/** Parsed diff section for a review comment's `file_path`, if the diff contains that file. */
export function diffFileForComment(comment: ReviewCommentEvaluation, files: ReadonlyArray<DiffFile>): DiffFile | undefined {
  if (!comment.file_path) return undefined;
  const target = normalizePath(comment.file_path);
  return files.find((f) => normalizePath(f.path) === target);
}
