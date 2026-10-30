import React, { useState } from "react";
import { Network } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import { Badge, Button, ErrorState, LoadingRows, Tabs, TabsContent, TabsList, TabsTrigger, TimeAgo } from "@symphony/ui";
import { DiffViewer } from "../../components/DiffViewer.js";
import { useMessages } from "../../lib/i18n.js";
import { useKnowledgeCommit, useKnowledgeCommitDiff } from "../../lib/queries.js";
import { AnalysisView } from "./AnalysisView.js";
import { ImpactView } from "./ImpactView.js";
import { Block, ShaChip } from "./parts.js";
import { isTestFile, worktreeLabel } from "./knowledgeLogic.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection, type CommitPanelTab } from "./selection.js";

const DiffTab: React.FC<{ sha: string }> = ({ sha }) => {
  const m = useMessages(knowledgeMessages);
  const { projectId } = useKnowledgeSelection();
  const diff = useKnowledgeCommitDiff(projectId, sha);
  if (diff.isPending) return <LoadingRows rows={6} />;
  if (diff.isError) return <ErrorState title={m.commits.loadFailed} message={diff.error.message} onRetry={() => void diff.refetch()} />;
  return (
    <DiffViewer
      diff={diff.data.diff}
      truncated={diff.data.truncated}
      truncatedLabel={formatMessage(m.commits.diffTruncated, { size: 512 })}
      emptyLabel={m.commits.noDiff}
    />
  );
};

const TestsTab: React.FC<{ sha: string; testFiles: ReadonlyArray<string> }> = ({ sha, testFiles }) => {
  const m = useMessages(knowledgeMessages);
  return (
    <div className="flex flex-col gap-4">
      <AnalysisView sha={sha} section="tests" />
      <Block title={m.tests.touched}>
        {testFiles.length === 0 ? (
          <p className="text-sm text-muted-foreground">{m.tests.none}</p>
        ) : (
          <ul role="list" className="flex flex-col gap-px">
            {testFiles.map((path) => (
              <li key={path} className="truncate rounded px-1.5 py-1 font-mono text-xs text-foreground" title={path}>
                {path}
              </li>
            ))}
          </ul>
        )}
      </Block>
    </div>
  );
};

/** One commit: who made it, where it lives, and the Diff / Impact / AI review / Tests views. */
export const CommitPanel: React.FC<{ sha: string }> = ({ sha }) => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const [tab, setTab] = useState<CommitPanelTab>("impact");
  const commit = useKnowledgeCommit(selection.projectId, sha);

  if (commit.isPending) return <LoadingRows className="p-3" rows={6} />;
  if (commit.isError) return <ErrorState title={m.commits.loadFailed} message={commit.error.message} onRetry={() => void commit.refetch()} />;
  const { commit: summary, body, files } = commit.data;
  // A rename counts when either side is a test file (a test moved out of a test directory).
  const testFiles = files.filter((f) => isTestFile(f.path) || (f.old_path != null && isTestFile(f.old_path))).map((f) => f.path);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 flex-col gap-2 border-b border-border p-3">
        <div className="flex items-start gap-2">
          <ShaChip sha={summary.sha} className="mt-0.5" />
          <h2 className="min-w-0 flex-1 text-sm font-semibold text-foreground [overflow-wrap:anywhere]">{summary.subject}</h2>
          <Button size="sm" variant="secondary" onClick={() => selection.selectCommit(sha, "graph")}>
            <Network className="size-4" />
            {m.commits.showInGraph}
          </Button>
        </div>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
          <span>
            {summary.author_name} · <TimeAgo iso={summary.committed_at} />
          </span>
          <span>{formatMessage(m.commits.files, { count: files.length, additions: commit.data.additions, deletions: commit.data.deletions })}</span>
          {summary.parents.length > 0 ? (
            <span className="flex items-center gap-1">
              {m.commits.parents}
              {summary.parents.map((parent) => (
                <button key={parent} type="button" onClick={() => selection.selectCommit(parent)} className="rounded outline-none focus-visible:ring-2 focus-visible:ring-ring/60">
                  <ShaChip sha={parent} />
                </button>
              ))}
            </span>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {summary.is_merge ? <Badge tone="info">{m.commits.badges.merge}</Badge> : null}
          {summary.unpushed ? <Badge tone="warning">{m.commits.badges.unpushed}</Badge> : null}
          {summary.on_default_branch ? <Badge tone="outline">{m.commits.badges.defaultBranch}</Badge> : null}
          {summary.refs.map((ref) => (
            <Badge key={ref} tone="accent">
              {ref}
            </Badge>
          ))}
          {summary.worktrees.map((wt) => (
            <Badge key={wt.worktree_id} tone="neutral" title={wt.is_head ? m.commits.badges.head : undefined}>
              {wt.branch ?? worktreeLabel({ branch: null, head_short_sha: null, path: wt.worktree_id })}
              {wt.is_head ? " ●" : ""}
            </Badge>
          ))}
        </div>
        {body ? (
          <details className="text-sm text-muted-foreground">
            <summary className="cursor-pointer text-xs">{m.commits.fullMessage}</summary>
            <pre className="mt-1 whitespace-pre-wrap font-sans text-sm">{body}</pre>
          </details>
        ) : null}
        {commit.data.files_truncated ? <p className="text-xs text-warning">{m.commits.filesTruncated}</p> : null}
      </header>
      <Tabs value={tab} onValueChange={(value) => setTab(value as CommitPanelTab)} className="min-h-0 flex-1">
        <TabsList aria-label={summary.subject}>
          {(["diff", "impact", "review", "tests"] as const).map((value) => (
            <TabsTrigger key={value} value={value}>
              {m.commits.panelTabs[value]}
            </TabsTrigger>
          ))}
        </TabsList>
        <div className="min-h-0 flex-1 overflow-auto p-3">
          <TabsContent value="diff">
            <DiffTab sha={sha} />
          </TabsContent>
          <TabsContent value="impact">
            <ImpactView sha={sha} />
          </TabsContent>
          <TabsContent value="review">
            <AnalysisView sha={sha} />
          </TabsContent>
          <TabsContent value="tests">
            <TestsTab sha={sha} testFiles={testFiles} />
          </TabsContent>
        </div>
      </Tabs>
    </div>
  );
};
