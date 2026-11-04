import React from "react";
import { FileCode2, GitCommitHorizontal, Package, Box, Braces, Boxes, CircleDashed, Cuboid, FunctionSquare } from "lucide-react";
import type { Confidence, EvidenceKind, EvidenceRef, NodeKind } from "@symphony/shared-types";
import { Badge, cn, type BadgeTone } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { knowledgeMessages } from "./messages.js";

const KIND_ICON: Record<NodeKind, React.ComponentType<{ className?: string }>> = {
  package: Package,
  module: Boxes,
  file: FileCode2,
  type: Cuboid,
  function: FunctionSquare,
  method: Braces,
  dependency: Box,
  other: CircleDashed
};

const KIND_TONE: Record<NodeKind, string> = {
  package: "text-accent",
  module: "text-accent",
  file: "text-info",
  type: "text-warning",
  function: "text-success",
  method: "text-success",
  dependency: "text-muted-foreground",
  other: "text-subtle-foreground"
};

/** Icon in the colour the graph draws that kind with. */
export const KindIcon: React.FC<{ kind: NodeKind; className?: string }> = ({ kind, className }) => {
  const Icon = KIND_ICON[kind];
  return <Icon aria-hidden className={cn("size-4 shrink-0", KIND_TONE[kind], className)} />;
};

export const KindBadge: React.FC<{ kind: NodeKind }> = ({ kind }) => {
  const m = useMessages(knowledgeMessages);
  return (
    <Badge tone="neutral">
      <KindIcon kind={kind} className="size-3" />
      {m.explorer.kinds[kind]}
    </Badge>
  );
};

const CONFIDENCE_TONE: Record<Confidence, BadgeTone> = { extracted: "success", inferred: "warning", ambiguous: "danger" };

export const ConfidenceBadge: React.FC<{ confidence: Confidence }> = ({ confidence }) => {
  const m = useMessages(knowledgeMessages);
  return <Badge tone={CONFIDENCE_TONE[confidence]}>{m.impact.confidence[confidence]}</Badge>;
};

/** A commit id in monospace. */
export const ShaChip: React.FC<{ sha: string; className?: string }> = ({ sha, className }) => (
  <code className={cn("rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground", className)}>{sha.slice(0, 8)}</code>
);

/** Evidence the model cited; opens the file's node, or the commit, when there is one to open. */
export const EvidenceChip: React.FC<{
  evidence: EvidenceRef;
  onOpenNode?: (nodeId: string) => void;
  onOpenCommit?: (sha: string) => void;
}> = ({ evidence, onOpenNode, onOpenCommit }) => {
  const m = useMessages(knowledgeMessages);
  const kind: EvidenceKind = evidence.kind;
  const open = evidence.node_id && onOpenNode ? () => onOpenNode(evidence.node_id!) : kind === "commit" && evidence.sha && onOpenCommit ? () => onOpenCommit(evidence.sha!) : null;
  const content = (
    <>
      {kind === "commit" ? <GitCommitHorizontal className="size-3" aria-hidden /> : <FileCode2 className="size-3" aria-hidden />}
      <span className="max-w-56 truncate">{evidence.label}</span>
      {evidence.line ? <span className="text-subtle-foreground">:{evidence.line}</span> : null}
    </>
  );
  const className = "inline-flex items-center gap-1 rounded border border-border bg-surface px-1.5 py-0.5 text-xs text-muted-foreground";
  const title = `${m.evidenceKinds[kind]}: ${evidence.file ?? evidence.label}`;
  return open ? (
    <button type="button" title={title} onClick={open} className={cn(className, "transition-colors hover:bg-surface-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60 outline-none")}>
      {content}
    </button>
  ) : (
    <span title={title} className={className}>
      {content}
    </span>
  );
};

/** A small titled block inside a panel. */
export const Block: React.FC<{ title?: React.ReactNode; actions?: React.ReactNode; children: React.ReactNode; className?: string }> = ({
  title,
  actions,
  children,
  className
}) => (
  <section className={cn("flex min-w-0 flex-col gap-2", className)}>
    {title || actions ? (
      <div className="flex items-center gap-2">
        {title ? <h3 className="min-w-0 flex-1 truncate text-xs font-semibold uppercase tracking-wide text-muted-foreground">{title}</h3> : <span className="flex-1" />}
        {actions}
      </div>
    ) : null}
    {children}
  </section>
);
