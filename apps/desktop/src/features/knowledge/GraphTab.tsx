import React, { Suspense, lazy, useMemo } from "react";
import { ArrowLeft, Network, X } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import type { GraphLevel, GraphNode } from "@symphony/shared-types";
import { AiEmptyView, Button, ErrorState, Label, LoadingRows, Spinner, Switch, ToggleGroup, ToggleGroupItem, cn } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useKnowledgeCommitImpact, useKnowledgeGraph, useKnowledgeOverview } from "../../lib/queries.js";
import { Explorer } from "./Explorer.js";
import { KindIcon, ShaChip } from "./parts.js";
import { highlightFor, locationLabel, type Mark } from "./knowledgeLogic.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

// The WebGL renderer is a separate chunk, loaded only when the graph is opened.
const GraphCanvas = lazy(() => import("./GraphCanvas.js"));

const LEVELS: ReadonlyArray<GraphLevel> = ["modules", "files", "symbols"];

const Legend: React.FC<{ active: boolean }> = ({ active }) => {
  const m = useMessages(knowledgeMessages);
  const item = (className: string, label: string) => (
    <li className="flex items-center gap-1.5">
      <span aria-hidden className={cn("size-2.5 rounded-full", className)} />
      {label}
    </li>
  );
  return (
    <ul aria-label={m.graph.legend.label} className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
      {active ? (
        <>
          {item("bg-warning", m.graph.legend.changed)}
          {item("bg-accent", m.graph.legend.affected)}
        </>
      ) : null}
      <li>{m.graph.legend.inferred}</li>
    </ul>
  );
};

/** The accessible twin of the canvas: every node of the view as a list, marked ones first. */
const NodeList: React.FC<{ nodes: ReadonlyArray<GraphNode>; marks: ReadonlyMap<string, Mark>; selectedId: string | null; onSelect: (node: GraphNode) => void }> = ({
  nodes,
  marks,
  selectedId,
  onSelect
}) => {
  const m = useMessages(knowledgeMessages);
  const ordered = useMemo(
    () =>
      [...nodes]
        .sort((a, b) => {
          const rank = (n: GraphNode) => (marks.get(n.id)?.emphasis === "changed" ? 0 : marks.get(n.id) ? 1 : 2);
          return rank(a) - rank(b) || b.degree - a.degree || a.label.localeCompare(b.label);
        })
        .slice(0, 300),
    [nodes, marks]
  );
  return (
    <ul aria-label={m.graph.listLabel} className="flex max-h-52 flex-col gap-px overflow-auto" role="list">
      {ordered.map((node) => {
        const mark = marks.get(node.id);
        return (
          <li key={node.id}>
            <button
              type="button"
              aria-current={node.id === selectedId ? "true" : undefined}
              onClick={() => onSelect(node)}
              className={cn(
                "flex w-full min-w-0 items-center gap-2 rounded px-1.5 py-1 text-start text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60",
                node.id === selectedId && "bg-surface-selected"
              )}
            >
              <KindIcon kind={node.kind} />
              <span className="min-w-0 flex-1 truncate">{node.label}</span>
              {mark ? <span className={cn("size-2 shrink-0 rounded-full", mark.emphasis === "changed" ? "bg-warning" : "bg-accent")} aria-hidden /> : null}
              <span className="hidden shrink-0 truncate font-mono text-xs text-subtle-foreground @2xl:inline">{locationLabel(node.file, node.line)}</span>
            </button>
          </li>
        );
      })}
    </ul>
  );
};

/** The code graph with the selected commit's changed and affected code highlighted on it. */
export const GraphTab: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const { projectId, level, module, nodeId, sha } = selection;
  const [includeInferred, setIncludeInferred] = React.useState(true);
  const [includeExternal, setIncludeExternal] = React.useState(false);

  const overview = useKnowledgeOverview(projectId);
  const indexed = overview.data?.index != null;
  const focus = level === "symbols" ? nodeId : null;
  const graph = useKnowledgeGraph(
    projectId,
    { level, module: level === "files" ? module : null, focus, depth: 1, includeInferred, includeExternal },
    indexed
  );
  const impact = useKnowledgeCommitImpact(projectId, sha);
  const nodes = graph.data?.nodes;
  const marks = useMemo(() => highlightFor(level, nodes ?? [], impact.data), [level, nodes, impact.data]);
  const changed = [...marks.values()].filter((mark) => mark.emphasis === "changed").length;
  const affected = marks.size - changed;

  const onSelect = (id: string | null) => {
    if (id === null) return selection.selectNode(null);
    const node = nodes?.find((n) => n.id === id);
    // A module is a place to go into, not an item with a detail view.
    if (level === "modules" && node && node.kind !== "dependency") return selection.openModule(id);
    selection.selectNode(id);
  };
  const onOpen = (id: string) => {
    const node = nodes?.find((n) => n.id === id);
    if (level === "modules" && node && node.kind !== "dependency") selection.openModule(id);
    else selection.focusNode(id);
  };

  if (overview.isPending) return <LoadingRows className="p-4" rows={6} />;
  if (!indexed) {
    return (
      <div className="p-4">
        <AiEmptyView icon={<Network className="size-5 text-accent" />} title={m.graph.needsIndex} description={overview.data?.active_run?.error ?? undefined} />
      </div>
    );
  }

  const levelLabel = m.graph.levels[level];
  const focusName = focus ? nodes?.find((n) => n.id === focus)?.label ?? focus : module ?? "";

  return (
    <div className="@container flex h-full min-h-0 flex-col gap-2 p-2 @4xl:flex-row">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <ToggleGroup type="single" aria-label={m.graph.levels.label} value={level} onValueChange={(next) => next && selection.setLevel(next as GraphLevel)}>
            {LEVELS.map((value) => (
              <ToggleGroupItem key={value} value={value}>
                {m.graph.levels[value]}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          {level !== "modules" ? (
            <Button size="sm" variant="subtle" onClick={() => selection.setLevel("modules")}>
              <ArrowLeft className="size-4" />
              {m.graph.back}
            </Button>
          ) : null}
          <div className="flex items-center gap-1.5">
            <Switch id="kb-inferred" checked={includeInferred} onCheckedChange={setIncludeInferred} />
            <Label htmlFor="kb-inferred" className="text-xs">
              {m.graph.includeInferred}
            </Label>
          </div>
          {level === "modules" ? (
            <div className="flex items-center gap-1.5">
              <Switch id="kb-external" checked={includeExternal} onCheckedChange={setIncludeExternal} />
              <Label htmlFor="kb-external" className="text-xs">
                {m.graph.includeExternal}
              </Label>
            </div>
          ) : null}
          {graph.isFetching ? <Spinner className="size-3.5" /> : null}
          <span className="flex-1" />
          {graph.data?.truncated ? (
            <span className="text-xs text-muted-foreground">{formatMessage(m.graph.truncated, { shown: graph.data.nodes.length, total: graph.data.total_nodes })}</span>
          ) : null}
        </div>

        {sha ? (
          <div role="status" className="flex flex-wrap items-center gap-2 rounded-md border border-border bg-accent-subtle px-2.5 py-1.5 text-sm">
            <ShaChip sha={sha} />
            <span className="min-w-0 flex-1">
              {impact.isPending
                ? <Spinner className="inline size-3.5" />
                : formatMessage(m.graph.commitBanner, { sha: sha.slice(0, 8), changed, affected })}
              {impact.data?.unavailable_reason ? <span className="ms-2 text-xs text-muted-foreground">{m.graph.commitNoSymbols}</span> : null}
            </span>
            <Button size="sm" variant="subtle" onClick={() => selection.selectCommit(null)}>
              <X className="size-4" />
              {m.graph.clearCommit}
            </Button>
          </div>
        ) : null}

        <div className="min-h-64 flex-1">
          {graph.isError ? (
            <ErrorState title={m.unavailable.title} message={graph.error.message} onRetry={() => void graph.refetch()} retryLabel={m.unavailable.retry} />
          ) : level === "symbols" && !focus ? (
            <AiEmptyView icon={<Network className="size-5 text-accent" />} title={m.graph.focusHint} />
          ) : graph.isPending ? (
            <LoadingRows rows={6} />
          ) : graph.data.nodes.length === 0 ? (
            <AiEmptyView icon={<Network className="size-5 text-accent" />} title={m.graph.empty} />
          ) : (
            <Suspense fallback={<LoadingRows rows={6} />}>
              <GraphCanvas
                nodes={graph.data.nodes}
                edges={graph.data.edges}
                marks={marks}
                selectedId={nodeId}
                onSelect={onSelect}
                onOpen={onOpen}
                labels={{ laying: m.graph.laying, failed: m.graph.failed, canvas: m.graph.canvasLabel, zoomIn: m.graph.zoomIn, zoomOut: m.graph.zoomOut, fit: m.graph.fit }}
              />
            </Suspense>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
          <Legend active={marks.size > 0} />
          {focusName ? <span className="truncate text-xs text-muted-foreground">{formatMessage(m.graph.selectedAt, { level: levelLabel, name: focusName })}</span> : null}
        </div>

        {graph.data && graph.data.nodes.length > 0 ? (
          <details className="@container rounded-md border border-border bg-surface px-2 py-1">
            <summary className="cursor-pointer text-sm text-muted-foreground">{m.graph.listLabel}</summary>
            <div className="pt-1">
              <NodeList nodes={graph.data.nodes} marks={marks} selectedId={nodeId} onSelect={(node) => onSelect(node.id)} />
            </div>
          </details>
        ) : null}
      </div>
      <div className="min-h-0 shrink-0 @4xl:w-80">
        <Explorer />
      </div>
    </div>
  );
};
