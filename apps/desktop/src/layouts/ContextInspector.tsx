import React from "react";
import { useNavigate } from "react-router-dom";
import { Button, Tooltip } from "@fluentui/react-components";
import { isTauri, nativeHost, errorMessage } from "@symphony/api-client";
import {
  AiEmptyView,
  KeyValueList,
  PhaseBadge,
  ProviderBadge,
  TaskStatusBadge,
  TimeAgo,
  formatCurrency,
  formatDuration,
  formatTokens,
  useNotify
} from "@symphony/ui";
import { getTranslations } from "@symphony/shared-types";
import { ExternalLink, FolderOpen, PanelRightClose, ScanSearch } from "lucide-react";
import { shellMessages } from "../app/messages.js";
import { paths } from "../app/routes.js";
import { useLocale, useMessages, useT } from "../lib/i18n.js";
import { useProjects, usePullRequest, useTask, useWorktrees } from "../lib/queries.js";
import { useRuntime } from "../runtime/RuntimeProvider.js";
import { useUiStore } from "../store/uiStore.js";

const Section: React.FC<{ title: string; actions?: React.ReactNode; children: React.ReactNode }> = ({ title, actions, children }) => (
  <section className="space-y-3 p-3">
    <div className="flex items-center gap-2">
      <h3 className="flex-1 truncate text-sm font-semibold text-foreground">{title}</h3>
      {actions}
    </div>
    {children}
  </section>
);

function useOpenWorkspace() {
  const notify = useNotify();
  const t = useT();
  const { mode } = useRuntime();
  const available = isTauri() && mode === "embedded";
  return {
    available,
    open: async (taskId: string) => {
      try {
        await nativeHost.openTaskWorkspace(taskId);
      } catch (e) {
        notify("error", t.errors.generic, errorMessage(e));
      }
    }
  };
}

const TaskInspector: React.FC<{ id: string }> = ({ id }) => {
  const m = useMessages(shellMessages);
  const locale = useLocale();
  const task = useTask(id);
  const navigate = useNavigate();
  const workspace = useOpenWorkspace();
  if (task.isError) return <p className="p-3 text-sm text-muted-foreground">{m.inspector.notFound}</p>;
  const t = task.data;
  if (!t) return null;
  return (
    <Section
      title={`${t.issue.identifier} · ${t.issue.title}`}
      actions={
        <Tooltip content={m.inspector.openExecution} relationship="label">
          <Button size="small" appearance="subtle" icon={<ScanSearch className="size-4" />} onClick={() => navigate(paths.run(t.id))} />
        </Tooltip>
      }
    >
      <div className="flex flex-wrap gap-1.5">
        <TaskStatusBadge status={t.status} />
        {t.current_phase ? <PhaseBadge phase={t.current_phase} /> : null}
        <ProviderBadge provider={t.provider} model={t.model} />
      </div>
      <KeyValueList
        items={[
          { label: m.inspector.branch, value: t.branch_name, mono: true },
          { label: m.inspector.workspace, value: t.workspace_path, mono: true },
          { label: m.inspector.attempts, value: t.attempt_count },
          { label: m.inspector.tokens, value: formatTokens(t.total_tokens, locale) },
          { label: m.inspector.cost, value: formatCurrency(t.estimated_cost_usd, locale) },
          { label: m.inspector.duration, value: formatDuration(t.duration_seconds, getTranslations(locale)) },
          { label: m.inspector.status, value: <TimeAgo iso={t.updated_at} /> }
        ]}
      />
      {t.last_error ? (
        <div className="rounded-md bg-destructive-subtle p-2 text-xs text-destructive">
          <div className="font-medium">{m.inspector.lastError}</div>
          <div className="break-words font-mono">{t.last_error}</div>
        </div>
      ) : null}
      <div className="flex flex-wrap gap-1.5">
        <Button size="small" appearance="primary" onClick={() => navigate(paths.run(t.id))}>
          {m.inspector.openExecution}
        </Button>
        {workspace.available && t.workspace_path ? (
          <Button size="small" icon={<FolderOpen className="size-4" />} onClick={() => void workspace.open(t.id)}>
            {m.inspector.openExplorer}
          </Button>
        ) : null}
      </div>
    </Section>
  );
};

const ProjectInspector: React.FC<{ id: string }> = ({ id }) => {
  const m = useMessages(shellMessages);
  const t = useT();
  const projects = useProjects();
  const navigate = useNavigate();
  const p = projects.data?.find((x) => x.id === id);
  if (!p) return projects.isLoading ? null : <p className="p-3 text-sm text-muted-foreground">{m.inspector.notFound}</p>;
  return (
    <Section
      title={p.name}
      actions={
        <Button size="small" appearance="subtle" icon={<ExternalLink className="size-4" />} onClick={() => navigate(paths.project(p.id))} aria-label={m.inspector.project} />
      }
    >
      <KeyValueList
        items={[
          { label: m.inspector.repository, value: p.repo_path, mono: true },
          { label: m.inspector.branch, value: p.default_branch, mono: true },
          { label: m.inspector.tracker, value: p.tracker_kind },
          { label: m.inspector.provider, value: <ProviderBadge provider={p.preferred_provider} model={p.preferred_model} /> },
          { label: m.inspector.concurrency, value: p.max_concurrent_agents },
          { label: m.inspector.autoDispatch, value: p.auto_dispatch ? t.common.yes : t.common.no }
        ]}
      />
    </Section>
  );
};

const WorktreeInspector: React.FC<{ id: string }> = ({ id }) => {
  const m = useMessages(shellMessages);
  const worktrees = useWorktrees();
  const navigate = useNavigate();
  const workspace = useOpenWorkspace();
  const w = worktrees.data?.find((x) => x.task_id === id);
  if (!w) return worktrees.isLoading ? null : <p className="p-3 text-sm text-muted-foreground">{m.inspector.notFound}</p>;
  return (
    <Section title={w.issue_identifier}>
      <div className="flex flex-wrap gap-1.5">
        <TaskStatusBadge status={w.task_status} />
        <ProviderBadge provider={w.provider} />
      </div>
      <KeyValueList
        items={[
          { label: m.inspector.workspace, value: w.workspace_path, mono: true },
          { label: m.inspector.branch, value: w.branch_name, mono: true },
          { label: m.inspector.repository, value: w.repo_path, mono: true },
          { label: m.inspector.status, value: <TimeAgo iso={w.last_activity_at} /> }
        ]}
      />
      <div className="flex flex-wrap gap-1.5">
        <Button size="small" appearance="primary" onClick={() => navigate(paths.run(w.task_id))}>
          {m.inspector.openExecution}
        </Button>
        {workspace.available && w.exists ? (
          <Button size="small" icon={<FolderOpen className="size-4" />} onClick={() => void workspace.open(w.task_id)}>
            {m.inspector.openExplorer}
          </Button>
        ) : null}
      </div>
    </Section>
  );
};

const ReviewInspector: React.FC<{ id: string }> = ({ id }) => {
  const m = useMessages(shellMessages);
  const pr = usePullRequest(id);
  const navigate = useNavigate();
  if (pr.isError) return <p className="p-3 text-sm text-muted-foreground">{m.inspector.notFound}</p>;
  const r = pr.data;
  if (!r) return null;
  return (
    <Section title={r.title}>
      <KeyValueList
        items={[
          { label: m.inspector.branch, value: `${r.branch_name} → ${r.base_branch}`, mono: true },
          { label: "CI", value: r.ci_status },
          { label: m.inspector.status, value: `${r.merge_readiness.score}/5 · ${r.merge_readiness.label}` }
        ]}
      />
      <Button size="small" appearance="primary" onClick={() => navigate(paths.review(r.id))}>
        {m.inspector.openReview}
      </Button>
    </Section>
  );
};

/** Right-hand context panel for the selected entity. */
export const ContextInspector: React.FC = () => {
  const m = useMessages(shellMessages);
  const target = useUiStore((s) => s.inspector);
  const setInspectorOpen = useUiStore((s) => s.setInspectorOpen);
  return (
    <aside aria-label={m.inspector.label} className="flex h-full min-w-0 flex-col border-s border-border bg-surface">
      <header className="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3">
        <h2 className="flex-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {target ? m.inspector[target.kind] : m.inspector.label}
        </h2>
        <Tooltip content={m.inspector.close} relationship="label">
          <Button size="small" appearance="subtle" icon={<PanelRightClose className="size-4" />} onClick={() => setInspectorOpen(false)} />
        </Tooltip>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {!target ? (
          <AiEmptyView className="m-3" title={m.inspector.empty} description={m.inspector.emptyDetail} />
        ) : target.kind === "task" ? (
          <TaskInspector id={target.id} />
        ) : target.kind === "project" ? (
          <ProjectInspector id={target.id} />
        ) : target.kind === "worktree" ? (
          <WorktreeInspector id={target.id} />
        ) : (
          <ReviewInspector id={target.id} />
        )}
      </div>
    </aside>
  );
};
