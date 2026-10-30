import React, { useState } from "react";
import { Network } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import type { ChangedSymbol, ImpactFile, SymbolChange } from "@symphony/shared-types";
import { Alert, Badge, Button, ErrorState, Label, LoadingRows, Switch, ToggleGroup, ToggleGroupItem, cn, type BadgeTone } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useKnowledgeCommitImpact } from "../../lib/queries.js";
import { Block, ConfidenceBadge, KindIcon, ShaChip } from "./parts.js";
import { locationLabel } from "./knowledgeLogic.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

const CHANGE_TONE: Record<SymbolChange, BadgeTone> = { added: "success", modified: "warning", removed: "danger" };

const SymbolRow: React.FC<{ symbol: ChangedSymbol }> = ({ symbol }) => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const open = symbol.node_id ? () => selection.focusNode(symbol.node_id!) : null;
  const content = (
    <>
      <KindIcon kind={symbol.kind} />
      <span className="min-w-0 flex-1 truncate">{symbol.label}</span>
      <span className="shrink-0 font-mono text-xs text-subtle-foreground">:{symbol.line}</span>
      <Badge tone={CHANGE_TONE[symbol.change]}>{m.impact.change[symbol.change]}</Badge>
    </>
  );
  return open ? (
    <button
      type="button"
      onClick={open}
      className="flex w-full min-w-0 items-center gap-2 rounded px-1.5 py-1 text-start text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60"
    >
      {content}
    </button>
  ) : (
    <div className="flex w-full min-w-0 items-center gap-2 px-1.5 py-1 text-sm">{content}</div>
  );
};

const FileImpact: React.FC<{ file: ImpactFile }> = ({ file }) => {
  const m = useMessages(knowledgeMessages);
  return (
    <div className="flex flex-col gap-0.5 rounded-md border border-border bg-surface p-1.5">
      <p className="truncate px-1.5 font-mono text-xs text-muted-foreground" title={file.path}>
        {file.path}
      </p>
      {file.symbols.map((symbol) => (
        <SymbolRow key={`${symbol.label}:${symbol.line}:${symbol.change}`} symbol={symbol} />
      ))}
      {file.unattributed_hunks > 0 ? (
        <p className="px-1.5 text-xs text-subtle-foreground">{formatMessage(m.impact.unattributed, { count: file.unattributed_hunks })}</p>
      ) : null}
    </div>
  );
};

/** What a commit changed at the symbol level and which code depends on it. */
export const ImpactView: React.FC<{ sha: string }> = ({ sha }) => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const [depth, setDepth] = useState(2);
  const [includeInferred, setIncludeInferred] = useState(true);
  const impact = useKnowledgeCommitImpact(selection.projectId, sha, depth, includeInferred);

  if (impact.isPending) return <LoadingRows rows={5} />;
  if (impact.isError) return <ErrorState title={m.commits.loadFailed} message={impact.error.message} onRetry={() => void impact.refetch()} />;
  const data = impact.data;
  const withSymbols = data.files.filter((file) => file.symbols.length > 0 || file.unattributed_hunks > 0);
  const anySymbols = data.files.some((file) => file.symbols.length > 0);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <Button size="sm" variant="secondary" onClick={() => selection.selectCommit(sha, "graph")}>
          <Network className="size-4" />
          {m.commits.showInGraph}
        </Button>
        <ToggleGroup type="single" aria-label={m.impact.depth} value={String(depth)} onValueChange={(next) => next && setDepth(Number(next))}>
          {[1, 2, 3, 4].map((value) => (
            <ToggleGroupItem key={value} value={String(value)}>
              {formatMessage(m.impact.hop, { depth: value })}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <div className="flex items-center gap-1.5">
          <Switch id="kb-impact-inferred" checked={includeInferred} onCheckedChange={setIncludeInferred} />
          <Label htmlFor="kb-impact-inferred" className="text-xs">
            {m.impact.includeInferred}
          </Label>
        </div>
      </div>

      {data.unavailable_reason ? (
        <Alert intent="info" title={formatMessage(m.impact.unavailable, { reason: data.unavailable_reason })} />
      ) : null}

      <Block title={m.impact.title}>
        {withSymbols.length > 0 ? (
          <div className="flex flex-col gap-2">
            {withSymbols.map((file) => (
              <FileImpact key={file.path} file={file} />
            ))}
          </div>
        ) : data.unavailable_reason ? (
          <ul role="list" className="flex flex-col gap-px">
            {data.files.map((file) => (
              <li key={file.path} className="truncate rounded px-1.5 py-1 font-mono text-xs text-muted-foreground" title={file.path}>
                {file.path}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">{m.impact.noSymbols}</p>
        )}
        {anySymbols ? <p className="text-xs text-subtle-foreground">{m.impact.precision}</p> : null}
        {data.symbol_files_skipped > 0 ? (
          <p className="text-xs text-warning">{formatMessage(m.impact.skippedFiles, { count: data.symbol_files_skipped })}</p>
        ) : null}
      </Block>

      <Block title={m.impact.affectedTitle}>
        {data.run_id === null ? null : data.affected.length === 0 ? (
          <p className="text-sm text-muted-foreground">{m.impact.affectedNone}</p>
        ) : (
          <>
            <p className="text-xs text-muted-foreground">{formatMessage(m.impact.affectedHint, { count: data.affected.length, depth: data.depth })}</p>
            <ul role="list" className="flex flex-col gap-px">
              {data.affected.map((item) => (
                <li key={item.node.id}>
                  <button
                    type="button"
                    onClick={() => selection.focusNode(item.node.id)}
                    className="flex w-full min-w-0 items-center gap-2 rounded px-1.5 py-1 text-start text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60"
                  >
                    <KindIcon kind={item.node.kind} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate">{item.node.label}</span>
                      <span className="block truncate font-mono text-xs text-subtle-foreground">{locationLabel(item.node.file, item.node.line)}</span>
                    </span>
                    <span className={cn("shrink-0 text-xs text-muted-foreground")}>
                      {formatMessage(m.impact.hop, { depth: item.depth })} · {formatMessage(m.impact.via, { relation: item.via_relation })}
                    </span>
                    {item.confidence !== "extracted" ? <ConfidenceBadge confidence={item.confidence} /> : null}
                  </button>
                </li>
              ))}
            </ul>
            {data.affected_truncated ? <p className="text-xs text-warning">{m.impact.truncated}</p> : null}
          </>
        )}
        {data.indexed_sha ? (
          <p className="flex items-center gap-1.5 text-xs text-subtle-foreground">
            {formatMessage(m.impact.indexedAt, { sha: "" })}
            <ShaChip sha={data.indexed_sha} />
          </p>
        ) : null}
      </Block>
    </div>
  );
};
