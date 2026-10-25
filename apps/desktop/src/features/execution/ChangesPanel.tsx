import React, { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Button, Tab, TabList } from "@fluentui/react-components";
import { ExternalLink } from "lucide-react";
import { formatMessage, getTranslations, type ChangedFileSummary, type Task, type ValidationResult } from "@symphony/shared-types";
import {
  AiEmptyView,
  Duration,
  ErrorState,
  KeyValueList,
  LoadingRows,
  MetricTile,
  SdkBadge,
  SdkCodeBlock,
  TimeAgo,
  cn,
  formatCurrency,
  formatDuration,
  formatTokens
} from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { DiffViewer } from "../../components/DiffViewer.js";
import { useLocale, useMessages, useT } from "../../lib/i18n.js";
import { usePullRequests, useTaskAttempts, useTaskDiff, useTaskValidations } from "../../lib/queries.js";
import { hasPullRequest } from "../reviews/reviewHelpers.js";
import { executionMessages } from "./messages.js";

type RightTab = "changes" | "validation" | "usage" | "pr";

/** Compact changed-files list with status and +/- counts. */
export const ChangedFilesList: React.FC<{ files: ReadonlyArray<ChangedFileSummary>; label: string }> = ({ files, label }) => (
  <ul aria-label={label} className="divide-y divide-border rounded-md border border-border">
    {files.map((f) => (
      <li key={f.path} className="flex items-center gap-2 px-2 py-1 text-xs">
        <span className="w-14 shrink-0 truncate text-muted-foreground">{f.status}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-foreground" title={f.path}>
          {f.path}
        </span>
        <span className="font-mono tabular-nums text-success">+{f.additions}</span>
        <span className="font-mono tabular-nums text-destructive">−{f.deletions}</span>
      </li>
    ))}
  </ul>
);

/** Changed files + unified diff for a task's worktree (shared with the Worktrees view). */
export const TaskDiffView: React.FC<{ taskId: string }> = ({ taskId }) => {
  const m = useMessages(executionMessages);
  const t = useT();
  const diff = useTaskDiff(taskId);
  if (diff.isLoading) return <LoadingRows rows={4} label={t.common.loading} />;
  if (diff.isError) return <ErrorState title={m.changes.loadFailed} message={diff.error.message} onRetry={() => void diff.refetch()} />;
  const d = diff.data;
  if (!d) return null;
  if (!d.exists) return <AiEmptyView title={m.changes.noWorktree} />;
  if (d.changed_files.length === 0 && !d.diff.trim()) return <AiEmptyView title={m.changes.noChanges} />;
  return (
    <div className="space-y-3">
      {d.changed_files.length > 0 ? <ChangedFilesList files={d.changed_files} label={m.changes.files} /> : null}
      <DiffViewer diff={d.diff} files={d.changed_files} truncated={d.truncated} truncatedLabel={m.changes.truncated} emptyLabel={m.changes.noChanges} />
    </div>
  );
};

const ValidationItem = React.memo(function ValidationItem({ result }: { result: ValidationResult }) {
  const m = useMessages(executionMessages);
  const locale = useLocale();
  const [open, setOpen] = useState(!result.passed);
  const output = [result.stdout, result.stderr].filter((s) => s.trim()).join("\n");
  return (
    <li className="space-y-1.5 rounded-md border border-border p-2">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <SdkBadge variant="soft" color={result.passed ? "success" : "danger"} size="sm" pill>
          {result.passed ? m.changes.passed : m.changes.failed}
        </SdkBadge>
        <code className="min-w-0 flex-1 truncate font-mono text-foreground" title={result.command}>
          {result.command}
        </code>
      </div>
      <div className="flex flex-wrap items-center gap-x-3 text-xs tabular-nums text-muted-foreground">
        <span>{formatDuration(result.duration_ms / 1000, getTranslations(locale))}</span>
        <span className={cn("font-mono", result.exit_code === 0 ? "text-success" : "text-destructive")}>
          {formatMessage(m.changes.exit, { code: result.exit_code })}
        </span>
        <span>{formatMessage(m.changes.attempt, { number: result.attempt_number })}</span>
        <TimeAgo iso={result.executed_at} />
        <Button className="ms-auto" size="small" appearance="subtle" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
          {open ? m.changes.hideOutput : m.changes.showOutput}
        </Button>
      </div>
      {open ? output ? <SdkCodeBlock language="text">{output}</SdkCodeBlock> : <p className="text-xs text-muted-foreground">{m.changes.noOutput}</p> : null}
    </li>
  );
});

const ValidationTab: React.FC<{ taskId: string }> = ({ taskId }) => {
  const m = useMessages(executionMessages);
  const t = useT();
  const validations = useTaskValidations(taskId);
  if (validations.isLoading) return <LoadingRows rows={3} label={t.common.loading} />;
  if (validations.isError) return <ErrorState title={t.errors.generic} message={validations.error.message} onRetry={() => void validations.refetch()} />;
  const list = [...(validations.data ?? [])].sort((a, b) => (a.executed_at < b.executed_at ? 1 : -1));
  if (list.length === 0) return <AiEmptyView title={m.changes.noValidations} description={m.changes.noValidationsDetail} />;
  return (
    <ul className="space-y-2">
      {list.map((v) => (
        <ValidationItem key={v.id} result={v} />
      ))}
    </ul>
  );
};

const UsageTab: React.FC<{ task: Task; liveSince?: string | null }> = ({ task, liveSince }) => {
  const m = useMessages(executionMessages);
  const t = useT();
  const locale = useLocale();
  const attempts = useTaskAttempts(task.id);
  const sorted = useMemo(() => [...(attempts.data ?? [])].sort((a, b) => a.attempt_number - b.attempt_number), [attempts.data]);
  return (
    <div className="space-y-3">
      <div className="grid grid-cols-2 gap-2">
        <MetricTile label={m.changes.inputTokens} value={formatTokens(task.input_tokens, locale)} />
        <MetricTile label={m.changes.outputTokens} value={formatTokens(task.output_tokens, locale)} />
        <MetricTile label={m.changes.totalTokens} value={formatTokens(task.total_tokens, locale)} tone="accent" />
        <MetricTile label={m.changes.cost} value={formatCurrency(task.estimated_cost_usd, locale)} />
        <MetricTile
          label={m.changes.duration}
          value={liveSince ? <Duration since={liveSince} /> : formatDuration(task.duration_seconds, getTranslations(locale))}
        />
      </div>
      <section className="space-y-1.5">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{m.changes.perAttempt}</h3>
        {attempts.isLoading ? (
          <LoadingRows rows={2} label={t.common.loading} />
        ) : sorted.length === 0 ? (
          <p className="text-sm text-muted-foreground">{m.context.noAttempts}</p>
        ) : (
          <KeyValueList
            items={sorted.map((a) => ({
              label: formatMessage(m.context.attempt, { number: a.attempt_number }),
              value: `${formatTokens(a.total_tokens, locale)} (${formatTokens(a.input_tokens, locale)} / ${formatTokens(a.output_tokens, locale)})`,
              mono: true
            }))}
          />
        )}
      </section>
    </div>
  );
};

const PullRequestTab: React.FC<{ task: Task }> = ({ task }) => {
  const m = useMessages(executionMessages);
  const t = useT();
  const navigate = useNavigate();
  const prs = usePullRequests(task.project_id);
  if (prs.isLoading) return <LoadingRows rows={3} label={t.common.loading} />;
  if (prs.isError) return <ErrorState title={t.errors.generic} message={prs.error.message} onRetry={() => void prs.refetch()} />;
  const pr = prs.data?.find((p) => p.task_id === task.id);
  if (!pr) return <AiEmptyView title={m.changes.noPr} description={m.changes.noPrDetail} />;
  const score = pr.merge_readiness.score;
  return (
    <section className="space-y-3 rounded-md border border-border p-3" aria-label={pr.title}>
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-foreground">{pr.title}</p>
          <p className="font-mono text-xs text-muted-foreground">
            {hasPullRequest(pr) ? formatMessage(m.changes.prNumber, { number: pr.pr_number }) : m.changes.localReview} · {pr.branch_name} → {pr.base_branch}
          </p>
        </div>
        <SdkBadge variant="soft" color={score >= 4 ? "success" : score >= 3 ? "warning" : "danger"} size="sm" pill>
          {score}/5
        </SdkBadge>
      </div>
      <KeyValueList
        items={[
          { label: m.changes.score, value: formatMessage(m.changes.scoreValue, { score, label: pr.merge_readiness.label }) },
          { label: m.changes.ci, value: pr.ci_status === "none" ? m.changes.noRemoteCi : pr.ci_status },
          { label: m.changes.state, value: pr.state },
          ...(pr.merge_readiness.blockers.length > 0
            ? [{ label: m.changes.blockersLabel, value: <span className="text-destructive">{formatMessage(m.changes.blockers, { count: pr.merge_readiness.blockers.length })}</span> }]
            : []),
          ...(pr.human_approved ? [{ label: m.changes.humanApproved, value: t.common.yes }] : [])
        ]}
      />
      <Button size="small" appearance="primary" icon={<ExternalLink className="size-4" />} onClick={() => navigate(paths.review(pr.id))}>
        {m.changes.openReview}
      </Button>
    </section>
  );
};

/** Right column: diff, validations, usage and pull request summary for the task. */
export const ChangesPanel: React.FC<{ task: Task; liveSince?: string | null }> = ({ task, liveSince }) => {
  const m = useMessages(executionMessages);
  const [tab, setTab] = useState<RightTab>("changes");
  return (
    <div className="flex h-full min-h-0 flex-col">
      <TabList
        size="small"
        selectedValue={tab}
        onTabSelect={(_, d) => setTab(d.value as RightTab)}
        aria-label={m.changes.tabs}
        className="shrink-0 flex-nowrap overflow-x-auto border-b border-border px-1 [scrollbar-width:none]"
      >
        <Tab value="changes">{m.changes.changes}</Tab>
        <Tab value="validation">{m.changes.validation}</Tab>
        <Tab value="usage">{m.changes.usage}</Tab>
        <Tab value="pr">{m.changes.pullRequest}</Tab>
      </TabList>
      <div role="tabpanel" className="min-h-0 flex-1 overflow-auto p-2">
        {tab === "changes" ? (
          <TaskDiffView taskId={task.id} />
        ) : tab === "validation" ? (
          <ValidationTab taskId={task.id} />
        ) : tab === "usage" ? (
          <UsageTab task={task} liveSince={liveSince} />
        ) : (
          <PullRequestTab task={task} />
        )}
      </div>
    </div>
  );
};
