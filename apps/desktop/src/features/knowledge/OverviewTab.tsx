import React from "react";
import { Check, CircleAlert, Copy, Info, TriangleAlert } from "lucide-react";
import type { EngineStatus, HealthSeverity, HealthSignal, ModuleStat } from "@symphony/shared-types";
import { Badge, Button, DataTable, ErrorState, KeyValueList, LoadingRows, MetricTile, Panel, TimeAgo, cn, useNotify, type ColumnDef } from "@symphony/ui";
import { useLocale, useMessages } from "../../lib/i18n.js";
import { useKnowledgeOverview } from "../../lib/queries.js";
import { Block, KindIcon, ShaChip } from "./parts.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

const HEALTH_ICON: Record<HealthSeverity, React.ReactNode> = {
  info: <Info aria-hidden className="size-4 shrink-0 text-info" />,
  warning: <TriangleAlert aria-hidden className="size-4 shrink-0 text-warning" />,
  problem: <CircleAlert aria-hidden className="size-4 shrink-0 text-destructive" />
};

const Health: React.FC<{ signals: ReadonlyArray<HealthSignal> }> = ({ signals }) => {
  const m = useMessages(knowledgeMessages);
  if (signals.length === 0) {
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <Check aria-hidden className="size-4 text-success" />
        {m.overview.healthy}
      </p>
    );
  }
  return (
    <ul role="list" className="flex flex-col gap-1.5">
      {signals.map((signal) => (
        <li key={signal.code} className="flex items-start gap-2 text-sm">
          {HEALTH_ICON[signal.severity]}
          <span className="min-w-0 flex-1 text-foreground [overflow-wrap:anywhere]">{signal.message}</span>
        </li>
      ))}
    </ul>
  );
};

const Engine: React.FC<{ engine: EngineStatus }> = ({ engine }) => {
  const m = useMessages(knowledgeMessages);
  const notify = useNotify();
  const copy = async () => {
    if (!engine.install_hint) return;
    try {
      await navigator.clipboard.writeText(engine.install_hint);
      notify("success", m.overview.copied);
    } catch {
      // Clipboard access can be refused; the command is shown on screen anyway.
    }
  };
  return (
    <li className="flex flex-col gap-1.5 rounded-md border border-border bg-surface p-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-medium text-foreground">{engine.name}</span>
        <Badge tone={engine.available ? "success" : engine.required ? "danger" : "neutral"}>
          {engine.available ? m.overview.engineAvailable : m.overview.engineMissing}
        </Badge>
        <Badge tone="outline">{engine.required ? m.overview.required : m.overview.optional}</Badge>
        {engine.version ? <span className="font-mono text-xs text-muted-foreground">{engine.version}</span> : null}
      </div>
      {engine.detail ? <p className="text-xs text-muted-foreground">{engine.detail}</p> : null}
      {!engine.available && engine.install_hint ? (
        <div className="flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded bg-code px-2 py-1 font-mono text-xs text-foreground" title={engine.install_hint}>
            {engine.install_hint}
          </code>
          <Button size="sm" variant="secondary" onClick={() => void copy()} aria-label={m.overview.copyInstall}>
            <Copy className="size-4" />
          </Button>
        </div>
      ) : null}
    </li>
  );
};

/** Languages, size, engines and health of the repository, and the way into its modules. */
export const OverviewTab: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const locale = useLocale();
  const selection = useKnowledgeSelection();
  const overview = useKnowledgeOverview(selection.projectId);

  if (overview.isPending) return <LoadingRows className="p-4" rows={6} />;
  if (overview.isError) {
    return (
      <div className="p-4">
        <ErrorState title={m.unavailable.title} message={overview.error.message} onRetry={() => void overview.refetch()} retryLabel={m.unavailable.retry} />
      </div>
    );
  }
  const data = overview.data;
  const index = data.index;
  const number = new Intl.NumberFormat(locale);
  const languageTotal = index?.languages.reduce((sum, l) => sum + l.files, 0) ?? 0;

  const columns: ColumnDef<ModuleStat, unknown>[] = [
    {
      id: "module",
      header: m.overview.moduleColumns.module,
      accessorFn: (r) => r.module,
      cell: ({ row }) => (
        <button
          type="button"
          className="max-w-[20rem] truncate rounded text-start font-mono text-xs text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/60"
          onClick={(event) => {
            event.stopPropagation();
            selection.openModule(row.original.module);
          }}
        >
          {row.original.module}
        </button>
      )
    },
    { id: "files", header: m.overview.moduleColumns.files, accessorFn: (r) => r.files, cell: ({ row }) => <span className="tabular-nums">{row.original.files}</span> },
    { id: "symbols", header: m.overview.moduleColumns.symbols, accessorFn: (r) => r.symbols, cell: ({ row }) => <span className="tabular-nums">{row.original.symbols}</span> },
    { id: "incoming", header: m.overview.moduleColumns.incoming, accessorFn: (r) => r.incoming, cell: ({ row }) => <span className="tabular-nums">{row.original.incoming}</span> },
    { id: "outgoing", header: m.overview.moduleColumns.outgoing, accessorFn: (r) => r.outgoing, cell: ({ row }) => <span className="tabular-nums">{row.original.outgoing}</span> }
  ];

  return (
    <div className="@container h-full overflow-auto p-3">
      <div className="mx-auto flex max-w-6xl flex-col gap-4">
        <div className="grid grid-cols-2 gap-2 @2xl:grid-cols-5">
          <MetricTile label={m.overview.metrics.files} value={index ? number.format(index.file_count) : "—"} />
          <MetricTile label={m.overview.metrics.symbols} value={index ? number.format(data.symbol_count) : "—"} />
          <MetricTile label={m.overview.metrics.edges} value={index ? number.format(index.edge_count) : "—"} />
          <MetricTile label={m.overview.metrics.worktrees} value={data.worktrees_total} />
          <MetricTile label={m.overview.metrics.dirty} value={data.worktrees_dirty} tone={data.worktrees_dirty > 0 ? "warning" : "neutral"} />
        </div>

        <div className="grid grid-cols-1 gap-4 @4xl:grid-cols-2">
          <Panel title={m.overview.indexTitle} bodyClassName="flex flex-col gap-3 p-3">
            {index ? (
              <KeyValueList
                items={[
                  { label: m.overview.rows.commit, value: <ShaChip sha={index.commit_sha} /> },
                  { label: m.overview.rows.ref, value: index.ref_name, mono: true },
                  { label: m.overview.rows.engine, value: `${index.engine} ${index.engine_version}`, mono: true },
                  { label: m.overview.rows.nodes, value: number.format(index.node_count) },
                  { label: m.overview.rows.edges, value: number.format(index.edge_count) },
                  { label: m.overview.rows.duration, value: index.duration_ms != null ? `${(index.duration_ms / 1000).toFixed(1)} s` : "—" },
                  { label: m.overview.rows.finished, value: <TimeAgo iso={index.finished_at} /> }
                ]}
              />
            ) : (
              <p className="text-sm text-muted-foreground">{m.overview.neverIndexed}</p>
            )}
            {index && index.languages.length > 0 ? (
              <Block title={m.overview.languagesTitle}>
                <ul role="list" className="flex flex-col gap-1.5">
                  {index.languages.slice(0, 8).map((language) => (
                    <li key={language.language} className="flex items-center gap-2 text-sm">
                      <span className="w-24 shrink-0 truncate text-foreground">{language.language}</span>
                      <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted" aria-hidden>
                        <span
                          className={cn("block h-full rounded-full bg-accent")}
                          style={{ width: `${languageTotal > 0 ? Math.max(2, Math.round((language.files / languageTotal) * 100)) : 0}%` }}
                        />
                      </span>
                      <span className="w-10 shrink-0 text-end tabular-nums text-muted-foreground">{language.files}</span>
                    </li>
                  ))}
                </ul>
              </Block>
            ) : null}
          </Panel>

          <div className="flex flex-col gap-4">
            <Panel title={m.overview.healthTitle} bodyClassName="p-3">
              <Health signals={data.health} />
            </Panel>
            <Panel title={m.overview.enginesTitle} bodyClassName="p-3">
              <ul role="list" className="flex flex-col gap-2">
                {data.engines.map((engine) => (
                  <Engine key={engine.id} engine={engine} />
                ))}
              </ul>
            </Panel>
          </div>
        </div>

        <Panel title={m.overview.modulesTitle} bodyClassName="p-0">
          {data.top_modules.length === 0 ? (
            <p className="p-3 text-sm text-muted-foreground">{m.overview.noGraph}</p>
          ) : (
            <div className="@container max-h-80 overflow-auto">
              <DataTable<ModuleStat>
                data={data.top_modules}
                columns={columns}
                getRowId={(r) => r.module}
                onRowActivate={(r) => selection.openModule(r.module)}
                ariaLabel={m.overview.modulesTitle}
              />
            </div>
          )}
        </Panel>

        {data.hubs.length > 0 ? (
          <Panel title={m.overview.hubsTitle} bodyClassName="p-2">
            <ul role="list" className="grid grid-cols-1 gap-px @2xl:grid-cols-2">
              {data.hubs.map((hub) => (
                <li key={hub.id}>
                  <button
                    type="button"
                    onClick={() => selection.focusNode(hub.id)}
                    className="flex w-full min-w-0 items-center gap-2 rounded px-1.5 py-1 text-start text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60"
                  >
                    <KindIcon kind={hub.kind} />
                    <span className="min-w-0 flex-1 truncate">{hub.label}</span>
                    <span className="shrink-0 tabular-nums text-xs text-muted-foreground">{hub.degree}</span>
                  </button>
                </li>
              ))}
            </ul>
          </Panel>
        ) : null}
      </div>
    </div>
  );
};
