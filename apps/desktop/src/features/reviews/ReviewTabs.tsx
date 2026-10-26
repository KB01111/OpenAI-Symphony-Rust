import React, { useMemo } from "react";
import { Button, MessageBar, MessageBarBody, Radio, RadioGroup, Tooltip } from "@fluentui/react-components";
import { CheckCircle2, FileDiff, ShieldAlert, X, XCircle } from "lucide-react";
import { formatMessage, type PullRequestReview, type ValidationResult } from "@symphony/shared-types";
import { AiEmptyView, ErrorState, LoadingRows, SdkBadge, SdkMarkdown, TimeAgo, cn, formatDuration } from "@symphony/ui";
import { errorMessage } from "@symphony/api-client";
import { DiffViewer, parseUnifiedDiff, type DiffFile } from "../../components/DiffViewer.js";
import { useT, useMessages } from "../../lib/i18n.js";
import { useTaskDiff } from "../../lib/queries.js";
import { reviewMessages } from "./messages.js";
import { diffFileForComment, diffForFile } from "./reviewHelpers.js";

const muted = "p-3 text-sm text-muted-foreground";

// ---------------------------------------------------------------------------------------------
// Files changed
// ---------------------------------------------------------------------------------------------

export const FilesChangedTab: React.FC<{
  review: PullRequestReview;
  onSelectFile: (path: string) => void;
}> = ({ review, onSelectFile }) => {
  const m = useMessages(reviewMessages);
  const inPreview = useMemo(() => new Set(parseUnifiedDiff(review.diff_preview).map((f) => f.path)), [review.diff_preview]);
  if (review.changed_files.length === 0) return <p className={muted}>{m.files.empty}</p>;
  return (
    <ul className="divide-y divide-border rounded-md border border-border" aria-label={m.tabs.files}>
      {review.changed_files.map((file) => (
        <li key={file.path}>
          <button
            type="button"
            onClick={() => onSelectFile(file.path)}
            aria-label={formatMessage(m.files.showDiff, { path: file.path })}
            className="flex h-row w-full items-center gap-2 px-2 text-start text-xs hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-accent"
          >
            <FileDiff className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
            <span className="min-w-0 flex-1 truncate font-mono text-foreground">{file.path}</span>
            {!inPreview.has(file.path) ? <span className="hidden text-subtle-foreground md:inline">{m.files.noHunk}</span> : null}
            <SdkBadge size="sm" variant="outline" color="secondary">
              {file.status}
            </SdkBadge>
            <span className="w-12 text-end font-mono text-success">+{file.additions}</span>
            <span className="w-12 text-end font-mono text-destructive">−{file.deletions}</span>
          </button>
        </li>
      ))}
    </ul>
  );
};

// ---------------------------------------------------------------------------------------------
// Diff (review snapshot or the task's live worktree)
// ---------------------------------------------------------------------------------------------

export type DiffSource = "snapshot" | "worktree";

const WorktreeDiff: React.FC<{ taskId: string; file: string | null }> = ({ taskId, file }) => {
  const m = useMessages(reviewMessages);
  const diff = useTaskDiff(taskId);
  const text = useMemo(() => (diff.data ? (file ? diffForFile(diff.data.diff, file) : diff.data.diff) : ""), [diff.data, file]);
  if (diff.isLoading) return <LoadingRows rows={6} />;
  if (diff.isError) return <ErrorState title={m.diff.worktreeError} message={errorMessage(diff.error)} onRetry={() => void diff.refetch()} />;
  if (!diff.data?.exists) return <p className={muted}>{m.diff.worktreeMissing}</p>;
  return (
    <DiffViewer
      diff={text}
      files={diff.data.changed_files}
      truncated={diff.data.truncated}
      truncatedLabel={m.diff.truncated}
      emptyLabel={file ? formatMessage(m.diff.fileNotInDiff, { path: file }) : m.diff.empty}
      expandFirst={file ? 1 : 3}
    />
  );
};

export const DiffTab: React.FC<{
  review: PullRequestReview;
  source: DiffSource;
  onSourceChange: (source: DiffSource) => void;
  file: string | null;
  onClearFile: () => void;
}> = ({ review, source, onSourceChange, file, onClearFile }) => {
  const m = useMessages(reviewMessages);
  const snapshot = useMemo(() => (file ? diffForFile(review.diff_preview, file) : review.diff_preview), [review.diff_preview, file]);
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-3">
        <RadioGroup
          layout="horizontal"
          aria-label={m.diff.source}
          value={source}
          onChange={(_, data) => onSourceChange(data.value as DiffSource)}
        >
          <Radio value="snapshot" label={m.diff.snapshot} />
          <Radio value="worktree" label={m.diff.worktree} />
        </RadioGroup>
        {file ? (
          <span className="inline-flex min-w-0 items-center gap-1 rounded-md border border-border bg-surface-sunken ps-2 text-xs">
            <span className="truncate font-mono">{formatMessage(m.diff.filteredTo, { path: file })}</span>
            <Tooltip content={m.diff.clearFile} relationship="label">
              <Button size="small" appearance="subtle" icon={<X className="size-3.5" />} onClick={onClearFile} />
            </Tooltip>
          </span>
        ) : null}
      </div>
      {source === "snapshot" ? (
        <>
          <p className="text-xs text-subtle-foreground">{m.diff.snapshotHint}</p>
          <DiffViewer
            diff={snapshot}
            files={review.changed_files}
            emptyLabel={file ? formatMessage(m.diff.fileNotInDiff, { path: file }) : m.diff.empty}
            expandFirst={file ? 1 : 3}
          />
        </>
      ) : (
        <WorktreeDiff taskId={review.task_id} file={file} />
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

const ValidationRow: React.FC<{ result: ValidationResult }> = ({ result }) => {
  const m = useMessages(reviewMessages);
  const t = useT();
  return (
    <details className="group rounded-md border border-border bg-surface">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2 py-1.5 text-xs hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-accent">
        {result.passed ? (
          <CheckCircle2 className="size-4 shrink-0 text-success" aria-label={m.validation.passed} />
        ) : (
          <XCircle className="size-4 shrink-0 text-destructive" aria-label={m.validation.failed} />
        )}
        <code className="min-w-0 flex-1 truncate font-mono text-foreground">{result.command}</code>
        <span className={cn("shrink-0", result.passed ? "text-success" : "text-destructive")}>
          {result.passed ? m.validation.passed : m.validation.failed}
        </span>
        <span className="shrink-0 text-muted-foreground">{formatMessage(m.validation.exitCode, { code: result.exit_code })}</span>
        <span className="shrink-0 tabular-nums text-muted-foreground">{formatDuration(result.duration_ms / 1000, t)}</span>
        <span className="hidden shrink-0 text-subtle-foreground sm:inline">{formatMessage(m.validation.attempt, { number: result.attempt_number })}</span>
        <TimeAgo iso={result.executed_at} className="shrink-0 text-subtle-foreground" />
      </summary>
      <div className="space-y-2 border-t border-border p-2">
        {(
          [
            [m.validation.stdout, result.stdout],
            [m.validation.stderr, result.stderr]
          ] as const
        ).map(([label, output]) => (
          <div key={label}>
            <div className="mb-1 text-xs font-medium text-muted-foreground">{label}</div>
            <pre tabIndex={0} className="max-h-64 overflow-auto rounded-md bg-code p-2 font-mono text-xs whitespace-pre-wrap text-foreground">
              {output.trim() ? output : m.validation.noOutput}
            </pre>
          </div>
        ))}
      </div>
    </details>
  );
};

export const ValidationTab: React.FC<{ results: ReadonlyArray<ValidationResult> }> = ({ results }) => {
  const m = useMessages(reviewMessages);
  if (results.length === 0) return <p className={muted}>{m.validation.empty}</p>;
  return (
    <div className="space-y-1.5">
      {results.map((r) => (
        <ValidationRow key={r.id} result={r} />
      ))}
    </div>
  );
};

// ---------------------------------------------------------------------------------------------
// AI review comments, each rendered next to the related file's diff hunk
// ---------------------------------------------------------------------------------------------

const COMMENT_DIFF_LINES = 200;

const lineClass = (line: string) =>
  line.startsWith("@@")
    ? "bg-info-subtle text-info"
    : line.startsWith("+")
      ? "bg-diff-add text-foreground"
      : line.startsWith("-")
        ? "bg-diff-remove text-foreground"
        : "text-muted-foreground";

const CommentFileDiff: React.FC<{ file: DiffFile }> = ({ file }) => {
  const m = useMessages(reviewMessages);
  const shown = file.lines.slice(0, COMMENT_DIFF_LINES);
  const label = formatMessage(m.comments.fileDiff, { path: file.path });
  return (
    <figure aria-label={label} className="min-w-0 overflow-hidden rounded-md border border-border">
      <figcaption className="flex items-center gap-2 bg-surface-sunken px-2 py-1 text-xs">
        <FileDiff className="size-3.5 text-muted-foreground" aria-hidden />
        <span className="min-w-0 flex-1 truncate font-mono text-foreground">{file.path}</span>
        <span className="font-mono text-success">+{file.additions}</span>
        <span className="font-mono text-destructive">−{file.deletions}</span>
      </figcaption>
      <div tabIndex={0} role="region" aria-label={file.path} className="max-h-72 overflow-auto bg-code font-mono text-xs leading-[18px]">
        {shown.map((line, i) => (
          <div key={i} className={cn("flex whitespace-pre px-2", lineClass(line))}>
            <span className="w-8 shrink-0 select-none pe-2 text-end text-subtle-foreground">{i + 1}</span>
            <span>{line || " "}</span>
          </div>
        ))}
      </div>
      {file.lines.length > COMMENT_DIFF_LINES ? (
        <p className="border-t border-border px-2 py-1 text-xs text-subtle-foreground">
          {formatMessage(m.comments.moreLines, { count: file.lines.length - COMMENT_DIFF_LINES })}
        </p>
      ) : null}
    </figure>
  );
};

export const CommentsTab: React.FC<{ review: PullRequestReview }> = ({ review }) => {
  const m = useMessages(reviewMessages);
  const files = useMemo(() => parseUnifiedDiff(review.diff_preview), [review.diff_preview]);
  if (review.review_comments.length === 0) return <p className={muted}>{m.comments.empty}</p>;
  return (
    <div className="space-y-3">
      {review.review_comments.map((comment, i) => {
        const file = diffFileForComment(comment, files);
        return (
          <article
            key={`${comment.author}-${i}`}
            aria-label={`${comment.author}${comment.file_path ? ` · ${comment.file_path}` : ""}`}
            className={cn("grid gap-2", file && "xl:grid-cols-2")}
          >
            <div className="min-w-0 space-y-2 rounded-md border border-border bg-surface p-3">
              <header className="flex flex-wrap items-center gap-1.5 text-xs">
                <span className="font-medium text-foreground">{comment.author}</span>
                <SdkBadge size="sm" variant="soft" color={comment.is_agent ? "discovery" : "secondary"} pill>
                  {comment.is_agent ? m.comments.agent : m.comments.human}
                </SdkBadge>
                <SdkBadge size="sm" variant="outline" color={comment.verified_against_diff ? "success" : "warning"} pill>
                  {comment.verified_against_diff ? m.comments.verified : m.comments.unverified}
                </SdkBadge>
                <span className="min-w-0 truncate font-mono text-muted-foreground">{comment.file_path ?? m.comments.general}</span>
              </header>
              <div className="text-sm text-foreground">
                <SdkMarkdown skipHtml>{comment.body}</SdkMarkdown>
              </div>
              {comment.evaluation_summary ? (
                <div className="rounded-md bg-surface-sunken px-2 py-1.5 text-xs">
                  <span className="font-medium text-muted-foreground">{m.comments.evaluation}: </span>
                  <span className="text-foreground">{comment.evaluation_summary}</span>
                </div>
              ) : null}
              {comment.file_path && !file ? <p className="text-xs text-subtle-foreground">{m.comments.fileNotInDiff}</p> : null}
            </div>
            {file ? <CommentFileDiff file={file} /> : null}
          </article>
        );
      })}
    </div>
  );
};

// ---------------------------------------------------------------------------------------------
// Security warnings
// ---------------------------------------------------------------------------------------------

export const SecurityTab: React.FC<{ warnings: ReadonlyArray<string> }> = ({ warnings }) => {
  const m = useMessages(reviewMessages);
  if (warnings.length === 0) {
    return <AiEmptyView title={m.security.empty} icon={<CheckCircle2 className="size-6 text-success" aria-hidden />} />;
  }
  return (
    <ul className="space-y-1.5">
      {warnings.map((warning, i) => (
        <li key={`${i}-${warning}`}>
          <MessageBar intent="warning" icon={<ShieldAlert className="size-4" aria-hidden />}>
            <MessageBarBody>{warning}</MessageBarBody>
          </MessageBar>
        </li>
      ))}
    </ul>
  );
};
