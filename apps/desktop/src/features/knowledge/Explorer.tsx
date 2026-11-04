import React, { useState } from "react";
import { Crosshair, Search } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import type { NodeNeighbor } from "@symphony/shared-types";
import { Button, ErrorState, Input, LoadingRows, Panel, Spinner, TimeAgo } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useKnowledgeNode, useKnowledgeNodeReferences, useKnowledgeSearch } from "../../lib/queries.js";
import { ConfidenceBadge, Block, KindBadge, KindIcon, ShaChip } from "./parts.js";
import { locationLabel, shortSubject } from "./knowledgeLogic.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

const LIST_LIMIT = 12;

const NeighborList: React.FC<{ items: ReadonlyArray<NodeNeighbor>; total: number; onSelect: (id: string) => void }> = ({ items, total, onSelect }) => {
  const m = useMessages(knowledgeMessages);
  return (
    <ul className="flex flex-col gap-px" role="list">
      {items.slice(0, LIST_LIMIT).map((item, index) => (
        <li key={`${item.node.id}:${item.relation}:${index}`}>
          <button
            type="button"
            onClick={() => onSelect(item.node.id)}
            className="flex w-full min-w-0 items-center gap-2 rounded px-1.5 py-1 text-start text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60"
          >
            <KindIcon kind={item.node.kind} />
            <span className="min-w-0 flex-1 truncate">{item.node.label}</span>
            <span className="shrink-0 text-xs text-subtle-foreground">{item.relation}</span>
            {item.confidence !== "extracted" ? <ConfidenceBadge confidence={item.confidence} /> : null}
          </button>
        </li>
      ))}
      {total > LIST_LIMIT ? <li className="px-1.5 text-xs text-muted-foreground">{formatMessage(m.explorer.more, { count: total - LIST_LIMIT })}</li> : null}
    </ul>
  );
};

/** References from the language server, fetched only when asked for. */
const PreciseReferences: React.FC<{ projectId: string | null; nodeId: string }> = ({ projectId, nodeId }) => {
  const m = useMessages(knowledgeMessages);
  const [requested, setRequested] = useState<string | null>(null);
  const references = useKnowledgeNodeReferences(projectId, nodeId, requested === nodeId);
  return (
    <Block title={m.explorer.precise.title}>
      {requested !== nodeId ? (
        <>
          <Button size="sm" variant="secondary" className="self-start" onClick={() => setRequested(nodeId)}>
            {m.explorer.precise.find}
          </Button>
          <p className="text-xs text-subtle-foreground">{m.explorer.precise.hint}</p>
        </>
      ) : references.isPending ? (
        <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
          <Spinner />
          {m.explorer.precise.loading}
        </p>
      ) : references.isError ? (
        <ErrorState title={m.explorer.loadFailed} message={references.error.message} />
      ) : !references.data.available ? (
        <p className="text-sm text-muted-foreground">{formatMessage(m.explorer.precise.unavailable, { reason: references.data.reason ?? "" })}</p>
      ) : references.data.references.length === 0 ? (
        <p className="text-sm text-muted-foreground">{m.explorer.precise.none}</p>
      ) : (
        <>
          {references.data.definition_start_line ? (
            <p className="text-xs text-subtle-foreground">
              {formatMessage(m.explorer.precise.definition, {
                start: references.data.definition_start_line,
                end: references.data.definition_end_line ?? references.data.definition_start_line
              })}
            </p>
          ) : null}
          <ul role="list" className="flex flex-col gap-1">
            {references.data.references.map((reference, index) => (
              <li key={`${reference.file}:${reference.name_path}:${reference.reference_line ?? index}`} className="rounded px-1.5 py-1 text-sm">
                <span className="block truncate font-medium" title={reference.name_path}>
                  {reference.name_path}
                </span>
                <span className="block truncate font-mono text-xs text-subtle-foreground">
                  {locationLabel(reference.file, reference.reference_line ?? reference.start_line)}
                </span>
              </li>
            ))}
          </ul>
          {references.data.truncated ? (
            <p className="text-xs text-muted-foreground">
              {formatMessage(m.explorer.precise.shown, { shown: references.data.references.length, total: references.data.total })}
            </p>
          ) : null}
        </>
      )}
    </Block>
  );
};

/** Search the index and inspect one node: what it uses, what uses it and how it recently changed. */
export const Explorer: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const [text, setText] = useState("");
  const search = useKnowledgeSearch(selection.projectId, text);
  const node = useKnowledgeNode(selection.projectId, selection.nodeId);
  const searching = text.trim().length > 0;

  return (
    <Panel title={m.explorer.title} className="h-full" bodyClassName="flex min-h-0 flex-col" aria-label={m.explorer.title}>
      <div className="relative shrink-0 border-b border-border p-2">
        <Search aria-hidden className="pointer-events-none absolute start-4 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={m.explorer.search}
          aria-label={m.explorer.searchLabel}
          className="ps-8"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-2">
        {searching ? (
          text.trim().length < 2 ? (
            <p className="px-1 text-sm text-muted-foreground">{m.explorer.minChars}</p>
          ) : search.isPending ? (
            <LoadingRows rows={4} />
          ) : search.isError ? (
            <ErrorState title={m.explorer.loadFailed} message={search.error.message} />
          ) : search.data.length === 0 ? (
            <p className="px-1 text-sm text-muted-foreground">{m.explorer.noResults}</p>
          ) : (
            <ul role="list" aria-label={m.explorer.searchLabel} className="flex flex-col gap-px">
              {search.data.map(({ node: hit }) => (
                <li key={hit.id}>
                  <button
                    type="button"
                    onClick={() => {
                      selection.selectNode(hit.id);
                      setText("");
                    }}
                    className="flex w-full min-w-0 items-center gap-2 rounded px-1.5 py-1 text-start text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60"
                  >
                    <KindIcon kind={hit.kind} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate">{hit.label}</span>
                      <span className="block truncate font-mono text-xs text-subtle-foreground">{locationLabel(hit.file, hit.line)}</span>
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )
        ) : selection.nodeId === null ? (
          <p className="px-1 text-sm text-muted-foreground">{m.explorer.none}</p>
        ) : node.isPending ? (
          <LoadingRows rows={5} />
        ) : node.isError ? (
          <ErrorState title={m.explorer.loadFailed} message={node.error.message} />
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex min-w-0 flex-col gap-1.5">
              <div className="flex items-center gap-2">
                <KindBadge kind={node.data.node.kind} />
                <h2 className="min-w-0 flex-1 truncate text-sm font-semibold text-foreground" title={node.data.node.label}>
                  {node.data.node.label}
                </h2>
              </div>
              {node.data.node.file ? (
                <p className="truncate font-mono text-xs text-muted-foreground" title={node.data.node.file}>
                  {locationLabel(node.data.node.file, node.data.node.line)}
                </p>
              ) : null}
              <dl className="grid grid-cols-3 gap-2 pt-1 text-center">
                {(
                  [
                    [m.explorer.stats.dependencies, node.data.dependencies_total],
                    [m.explorer.stats.references, node.data.references_total],
                    [m.explorer.stats.changes, node.data.recent_commits.length]
                  ] as const
                ).map(([label, value]) => (
                  <div key={label} className="rounded-md border border-border bg-surface-sunken px-2 py-1.5">
                    <dd className="font-display text-base font-semibold tabular-nums">{value}</dd>
                    <dt className="truncate text-xs text-muted-foreground">{label}</dt>
                  </div>
                ))}
              </dl>
              <Button size="sm" variant="secondary" onClick={() => selection.focusNode(node.data.node.id)} className="self-start">
                <Crosshair className="size-4" />
                {m.explorer.focus}
              </Button>
            </div>
            {node.data.members.length > 0 ? (
              <Block title={m.explorer.contains}>
                <ul role="list" className="flex flex-col gap-px">
                  {node.data.members.slice(0, LIST_LIMIT).map((member) => (
                    <li key={member.id}>
                      <button
                        type="button"
                        onClick={() => selection.selectNode(member.id)}
                        className="flex w-full min-w-0 items-center gap-2 rounded px-1.5 py-1 text-start text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60"
                      >
                        <KindIcon kind={member.kind} />
                        <span className="min-w-0 flex-1 truncate">{member.label}</span>
                        {member.line ? <span className="shrink-0 font-mono text-xs text-subtle-foreground">:{member.line}</span> : null}
                      </button>
                    </li>
                  ))}
                  {node.data.members.length > LIST_LIMIT ? (
                    <li className="px-1.5 text-xs text-muted-foreground">{formatMessage(m.explorer.more, { count: node.data.members.length - LIST_LIMIT })}</li>
                  ) : null}
                </ul>
              </Block>
            ) : null}
            {node.data.dependencies.length > 0 ? (
              <Block title={m.explorer.uses}>
                <NeighborList items={node.data.dependencies} total={node.data.dependencies_total} onSelect={(id) => selection.selectNode(id)} />
              </Block>
            ) : null}
            {node.data.references.length > 0 ? (
              <Block title={m.explorer.usedBy}>
                <NeighborList items={node.data.references} total={node.data.references_total} onSelect={(id) => selection.selectNode(id)} />
              </Block>
            ) : null}
            {node.data.node.file && ["type", "function", "method"].includes(node.data.node.kind) ? (
              <PreciseReferences key={node.data.node.id} projectId={selection.projectId} nodeId={node.data.node.id} />
            ) : null}
            {node.data.recent_commits.length > 0 ? (
              <Block title={m.explorer.recent}>
                <ul role="list" className="flex flex-col gap-px">
                  {node.data.recent_commits.slice(0, 6).map((commit) => (
                    <li key={commit.sha}>
                      <button
                        type="button"
                        title={m.explorer.openCommit}
                        onClick={() => selection.selectCommit(commit.sha, "commits")}
                        className="flex w-full min-w-0 items-center gap-2 rounded px-1.5 py-1 text-start text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60"
                      >
                        <ShaChip sha={commit.sha} />
                        <span className="min-w-0 flex-1 truncate">{shortSubject(commit.subject)}</span>
                        <TimeAgo iso={commit.committed_at} className="shrink-0 text-xs text-subtle-foreground" />
                      </button>
                    </li>
                  ))}
                </ul>
              </Block>
            ) : null}
          </div>
        )}
      </div>
    </Panel>
  );
};
