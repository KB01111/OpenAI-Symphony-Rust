import React from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { Badge, Button, StatusDot, TimeAgo, cn } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useKnowledgeCommits } from "../../lib/queries.js";
import { ShaChip } from "./parts.js";
import { shortSubject } from "./knowledgeLogic.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

const STRIP_COMMITS = 14;

/**
 * The newest commits across all worktrees, always visible under the tabs. Choosing one selects it
 * everywhere: the graph highlights what it changed and the commit panel opens on it.
 */
export const CommitStrip: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const [open, setOpen] = React.useState(true);
  const timeline = useKnowledgeCommits(selection.projectId, 100);
  const commits = timeline.data?.commits.slice(0, STRIP_COMMITS) ?? [];

  return (
    <section aria-label={m.commits.listLabel} className="shrink-0 border-t border-border bg-surface">
      <div className="flex items-center gap-2 px-3 py-1">
        <StatusDot tone="accent" pulse label={m.commits.listLabel} />
        <h2 className="flex-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{m.commits.listLabel}</h2>
        <Button size="sm" variant="subtle" aria-expanded={open} aria-label={m.commits.listLabel} onClick={() => setOpen((v) => !v)}>
          {open ? <ChevronDown className="size-4" /> : <ChevronUp className="size-4" />}
        </Button>
      </div>
      {open ? (
        commits.length === 0 ? null : (
          <ul role="list" className="flex gap-2 overflow-x-auto px-3 pb-2 [scrollbar-width:thin]">
            {commits.map((commit) => {
              const selected = commit.sha === selection.sha;
              return (
                <li key={commit.sha} className="shrink-0">
                  <button
                    type="button"
                    onClick={() => selection.selectCommit(selected ? null : commit.sha)}
                    aria-pressed={selected}
                    className={cn(
                      "flex w-60 flex-col gap-1 rounded-md border px-2.5 py-1.5 text-start outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60",
                      selected ? "border-accent bg-surface-selected" : "border-border bg-surface-raised"
                    )}
                  >
                    <span className="flex items-center gap-2">
                      <ShaChip sha={commit.sha} />
                      <span className="min-w-0 flex-1 truncate text-sm text-foreground">{shortSubject(commit.subject, 60)}</span>
                    </span>
                    <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                      <TimeAgo iso={commit.committed_at} />
                      {commit.worktrees.slice(0, 2).map((wt) => (
                        <Badge key={wt.worktree_id} tone="neutral" className="max-w-24 truncate">
                          {wt.branch ?? "detached"}
                        </Badge>
                      ))}
                      {commit.unpushed ? <Badge tone="warning">{m.commits.badges.unpushed}</Badge> : null}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )
      ) : null}
    </section>
  );
};
