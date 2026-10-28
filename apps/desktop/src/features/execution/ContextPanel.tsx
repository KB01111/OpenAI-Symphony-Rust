import React from "react";
import { formatMessage, getTranslations, type Project, type ProviderStatus, type RunAttempt, type Task } from "@symphony/shared-types";
import {
  ErrorState,
  KeyValueList,
  LoadingRows,
  PhaseBadge,
  ProviderBadge,
  CopyButton,
  Markdown,
  TimeAgo,
  cn,
  formatDuration,
  formatTokens
} from "@symphony/ui";
import { useLocale, useMessages, useT } from "../../lib/i18n.js";
import { useTaskAttempts, useWorkflow } from "../../lib/queries.js";
import { executionMessages } from "./messages.js";

const Section: React.FC<{ title: string; children: React.ReactNode }> = ({ title, children }) => (
  <section className="space-y-2 border-b border-border p-3 last:border-b-0">
    <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{title}</h3>
    {children}
  </section>
);

function policyText(value: unknown): string {
  if (value === null || value === undefined) return "—";
  return typeof value === "string" ? value : JSON.stringify(value);
}

const AttemptItem = React.memo(function AttemptItem({ attempt }: { attempt: RunAttempt }) {
  const m = useMessages(executionMessages);
  const locale = useLocale();
  return (
    <li className="space-y-1 rounded-md border border-border bg-surface-sunken p-2 text-xs">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="font-medium text-foreground">{formatMessage(m.context.attempt, { number: attempt.attempt_number })}</span>
        <PhaseBadge phase={attempt.phase} />
        <span className="ms-auto tabular-nums text-muted-foreground">
          {formatMessage(m.context.tokens, { count: formatTokens(attempt.total_tokens, locale) })}
        </span>
      </div>
      <div className="flex flex-wrap gap-x-3 gap-y-0.5 text-muted-foreground">
        <span>
          {m.context.startedAt} <TimeAgo iso={attempt.started_at} />
        </span>
        <span>
          {m.context.finishedAt} {attempt.finished_at ? <TimeAgo iso={attempt.finished_at} /> : m.context.inProgress}
        </span>
        <span>{formatMessage(m.context.turns, { count: attempt.turn_count })}</span>
      </div>
      {attempt.error ? <p className="break-words font-mono text-destructive">{attempt.error}</p> : null}
    </li>
  );
});

const AttemptsSection: React.FC<{ taskId: string }> = ({ taskId }) => {
  const m = useMessages(executionMessages);
  const t = useT();
  const attempts = useTaskAttempts(taskId);
  return (
    <Section title={m.context.attempts}>
      {attempts.isLoading ? (
        <LoadingRows rows={2} label={t.common.loading} />
      ) : attempts.isError ? (
        <ErrorState title={t.errors.generic} message={attempts.error.message} onRetry={() => void attempts.refetch()} />
      ) : (attempts.data ?? []).length === 0 ? (
        <p className="text-sm text-muted-foreground">{m.context.noAttempts}</p>
      ) : (
        <ul className="space-y-1.5">
          {[...(attempts.data ?? [])]
            .sort((a, b) => b.attempt_number - a.attempt_number)
            .map((a) => (
              <AttemptItem key={a.id} attempt={a} />
            ))}
        </ul>
      )}
    </Section>
  );
};

const WorkflowSection: React.FC = () => {
  const m = useMessages(executionMessages);
  const t = useT();
  const locale = useLocale();
  const workflow = useWorkflow();
  const dict = getTranslations(locale);
  const ms = (value: number) => formatDuration(value / 1000, dict);
  const doc = workflow.data;
  const config = doc?.config;
  return (
    <Section title={m.context.workflow}>
      {workflow.isLoading ? (
        <LoadingRows rows={3} label={t.common.loading} />
      ) : workflow.isError ? (
        <ErrorState title={t.errors.generic} message={workflow.error.message} onRetry={() => void workflow.refetch()} />
      ) : (
        <>
          {doc && !doc.valid ? <ErrorState intent="warning" title={m.context.workflowInvalid} message={doc.error ?? undefined} /> : null}
          <KeyValueList
            items={[
              { label: m.context.workflowPath, value: doc?.path ?? m.context.workflowDefault, mono: Boolean(doc?.path) },
              ...(config
                ? [
                    { label: m.context.maxTurns, value: config.agent.max_turns },
                    { label: m.context.maxAgents, value: config.agent.max_concurrent_agents },
                    { label: m.context.approvalPolicy, value: policyText(config.codex.approval_policy), mono: true },
                    { label: m.context.sandbox, value: config.codex.thread_sandbox, mono: true },
                    { label: m.context.turnTimeout, value: ms(config.codex.turn_timeout_ms) },
                    { label: m.context.readTimeout, value: ms(config.codex.read_timeout_ms) },
                    { label: m.context.stallTimeout, value: ms(config.codex.stall_timeout_ms) }
                  ]
                : [])
            ]}
          />
        </>
      )}
    </Section>
  );
};

const CapabilityRow: React.FC<{ label: string; supported: boolean }> = ({ label, supported }) => {
  const m = useMessages(executionMessages);
  return (
    <li className="flex items-center gap-2 text-sm">
      <span className={cn("size-1.5 shrink-0 rounded-full", supported ? "bg-success" : "bg-muted-foreground")} aria-hidden />
      <span className="flex-1 text-foreground">{label}</span>
      <span className={cn("text-xs", supported ? "text-success" : "text-muted-foreground")}>{supported ? m.context.supported : m.context.unsupported}</span>
    </li>
  );
};

const ExecutionSettings: React.FC<{ status: ProviderStatus | undefined }> = ({ status }) => {
  const m = useMessages(executionMessages);
  if (!status) return <p className="text-sm text-muted-foreground">{m.context.providerUnavailable}</p>;
  const c = status.capabilities;
  return (
    <div className="space-y-2">
      <KeyValueList
        items={[
          { label: m.context.installed, value: status.installed ? m.context.installed : m.context.notInstalled },
          { label: m.context.version, value: status.version ?? "—", mono: true },
          { label: m.context.binary, value: status.binary_path ?? "—", mono: true }
        ]}
      />
      <ul className="space-y-1" aria-label={m.context.execution}>
        <CapabilityRow label={m.context.capability.streaming} supported={c.supports_streaming_events} />
        <CapabilityRow label={m.context.capability.approvals} supported={c.supports_approvals} />
        <CapabilityRow label={m.context.capability.cancel} supported={c.supports_cancel} />
        <CapabilityRow label={m.context.capability.pause} supported={c.supports_mid_turn_pause} />
        <CapabilityRow label={m.context.capability.resume} supported={c.supports_session_resume} />
        <CapabilityRow label={m.context.capability.tools} supported={c.supports_dynamic_tools} />
        <CapabilityRow label={m.context.capability.tokens} supported={c.supports_token_usage} />
      </ul>
      {c.status_message ? <p className="text-xs text-muted-foreground">{c.status_message}</p> : null}
    </div>
  );
};

export interface ContextPanelProps {
  task: Task;
  project: Project | undefined;
  providerStatus: ProviderStatus | undefined;
}

/** Left column of the execution workspace: task, worktree, attempts, workflow and provider settings. */
export const ContextPanel: React.FC<ContextPanelProps> = ({ task, project, providerStatus }) => {
  const m = useMessages(executionMessages);
  const locale = useLocale();
  const description = task.issue.description?.trim();
  return (
    <div className="h-full overflow-auto">
      <Section title={m.context.task}>
        <div className="text-sm text-foreground">
          {description ? (
            <Markdown breakNewLines>{description}</Markdown>
          ) : (
            <p className="text-muted-foreground">{m.context.noDescription}</p>
          )}
        </div>
        <KeyValueList
          items={[
            { label: m.context.project, value: project?.name ?? m.unknownProject },
            { label: m.context.provider, value: <ProviderBadge provider={task.provider} /> },
            { label: m.context.model, value: task.model ?? providerStatus?.default_model ?? m.context.defaultModel, mono: true },
            { label: m.context.branch, value: task.branch_name ?? "—", mono: true },
            {
              label: m.context.worktree,
              mono: true,
              value: task.workspace_path ? (
                <span className="flex items-start gap-1">
                  <span className="min-w-0 flex-1 break-all">{task.workspace_path}</span>
                  <CopyButton value={task.workspace_path} label={m.context.copyWorktree} />
                </span>
              ) : (
                "—"
              )
            },
            { label: m.context.session, value: task.current_session_id ?? "—", mono: true },
            { label: m.context.thread, value: task.current_thread_id ?? "—", mono: true },
            { label: m.context.turn, value: task.current_turn_id ?? "—", mono: true },
            ...(task.next_retry_at
              ? [{ label: m.context.nextRetry, value: <time dateTime={task.next_retry_at}>{new Date(task.next_retry_at).toLocaleString(locale)}</time> }]
              : [])
          ]}
        />
        {task.last_error ? (
          <div className="rounded-md bg-destructive-subtle p-2 text-xs text-destructive">
            <div className="font-medium">{m.context.lastError}</div>
            <div className="break-words font-mono">{task.last_error}</div>
          </div>
        ) : null}
      </Section>
      <AttemptsSection taskId={task.id} />
      <WorkflowSection />
      <Section title={m.context.execution}>
        <ExecutionSettings status={providerStatus} />
      </Section>
    </div>
  );
};
