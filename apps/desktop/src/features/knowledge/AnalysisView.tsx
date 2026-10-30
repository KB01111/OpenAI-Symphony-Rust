import React from "react";
import { RefreshCw, Sparkles } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import type { AnalysisFinding, CommitAnalysis, EvidenceRef, FindingSeverity } from "@symphony/shared-types";
import { AiEmptyView, Badge, Button, ErrorState, LoadingRows, Spinner, formatTokens, type BadgeTone } from "@symphony/ui";
import { useLocale, useMessages } from "../../lib/i18n.js";
import { useKnowledgeCommitAnalysis, useStartKnowledgeAnalysis } from "../../lib/queries.js";
import { Block, EvidenceChip } from "./parts.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

const SEVERITY_TONE: Record<FindingSeverity, BadgeTone> = { info: "neutral", low: "info", medium: "warning", high: "danger", critical: "danger" };

const Evidence: React.FC<{ ids: ReadonlyArray<string>; evidence: ReadonlyArray<EvidenceRef> }> = ({ ids, evidence }) => {
  const selection = useKnowledgeSelection();
  const byId = new Map(evidence.map((e) => [e.id, e]));
  const refs = ids.map((id) => byId.get(id)).filter((e): e is EvidenceRef => e !== undefined);
  if (refs.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1">
      {refs.map((ref) => (
        <EvidenceChip
          key={ref.id}
          evidence={ref}
          onOpenNode={(nodeId) => selection.focusNode(nodeId)}
          onOpenCommit={(sha) => selection.selectCommit(sha, "commits")}
        />
      ))}
    </div>
  );
};

const Findings: React.FC<{ title: string; findings: ReadonlyArray<AnalysisFinding>; evidence: ReadonlyArray<EvidenceRef> }> = ({ title, findings, evidence }) => {
  const m = useMessages(knowledgeMessages);
  return (
    <Block title={title}>
      {findings.length === 0 ? (
        <p className="text-sm text-muted-foreground">{m.analysis.empty}</p>
      ) : (
        <ul role="list" className="flex flex-col gap-2">
          {findings.map((finding, index) => (
            <li key={`${finding.title}:${index}`} className="flex flex-col gap-1.5 rounded-md border border-border bg-surface p-2.5">
              <div className="flex items-start gap-2">
                <h4 className="min-w-0 flex-1 text-sm font-medium text-foreground">{finding.title}</h4>
                <Badge tone={SEVERITY_TONE[finding.severity]}>{m.analysis.severity[finding.severity]}</Badge>
              </div>
              {finding.detail ? <p className="whitespace-pre-wrap text-sm text-muted-foreground">{finding.detail}</p> : null}
              <Evidence ids={finding.evidence} evidence={evidence} />
            </li>
          ))}
        </ul>
      )}
    </Block>
  );
};

/** The AI review of one commit: run it, watch it, read findings that each cite their evidence. */
export const AnalysisView: React.FC<{ sha: string; section?: "review" | "tests" }> = ({ sha, section = "review" }) => {
  const m = useMessages(knowledgeMessages);
  const locale = useLocale();
  const selection = useKnowledgeSelection();
  const query = useKnowledgeCommitAnalysis(selection.projectId, sha);
  const start = useStartKnowledgeAnalysis(selection.projectId);
  const analysis: CommitAnalysis | null | undefined = query.data?.analysis;
  const running = analysis?.status === "running" || start.isPending;

  if (query.isPending) return <LoadingRows rows={4} />;
  if (query.isError) return <ErrorState title={m.commits.loadFailed} message={query.error.message} onRetry={() => void query.refetch()} />;

  const runButton = (label: string, force: boolean) => (
    <Button size="sm" variant={force ? "secondary" : "default"} disabled={running} onClick={() => start.mutate({ sha, force })}>
      {running ? <Spinner /> : force ? <RefreshCw className="size-4" /> : <Sparkles className="size-4" />}
      {label}
    </Button>
  );

  if (!analysis) {
    return section === "tests" ? (
      <p className="text-sm text-muted-foreground">{m.tests.runFirst}</p>
    ) : (
      <AiEmptyView icon={<Sparkles className="size-5 text-accent" />} title={m.analysis.none} description={m.analysis.noneHint} action={runButton(m.analysis.run, false)} />
    );
  }
  if (analysis.status === "running") {
    return (
      <div role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner />
        {m.analysis.running}
      </div>
    );
  }
  if (analysis.status === "failed") {
    return <ErrorState title={m.analysis.failed} message={analysis.error ?? undefined} onRetry={() => (running ? undefined : start.mutate({ sha, force: true }))} retryLabel={m.analysis.rerun} />;
  }
  const result = analysis.result;
  if (!result) return null;

  if (section === "tests") {
    return (
      <Block title={m.tests.suggested}>
        {result.tests.length === 0 ? (
          <p className="text-sm text-muted-foreground">{m.analysis.empty}</p>
        ) : (
          <ul role="list" className="flex flex-col gap-2">
            {result.tests.map((test, index) => (
              <li key={`${test.title}:${index}`} className="flex flex-col gap-1.5 rounded-md border border-border bg-surface p-2.5">
                <h4 className="text-sm font-medium text-foreground">{test.title}</h4>
                {test.rationale ? <p className="whitespace-pre-wrap text-sm text-muted-foreground">{test.rationale}</p> : null}
                <Evidence ids={test.evidence} evidence={analysis.evidence} />
              </li>
            ))}
          </ul>
        )}
      </Block>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>
          {formatMessage(m.analysis.by, { provider: analysis.provider, model: analysis.model ? ` · ${analysis.model}` : "" })}
        </span>
        <span>{formatMessage(m.analysis.tokens, { input: formatTokens(analysis.input_tokens, locale), output: formatTokens(analysis.output_tokens, locale) })}</span>
        <span className="flex-1" />
        {runButton(m.analysis.rerun, true)}
      </div>
      <Block title={m.analysis.summary}>
        <p className="whitespace-pre-wrap text-sm text-foreground">{result.summary}</p>
      </Block>
      <Findings title={m.analysis.sections.what_changed} findings={result.what_changed} evidence={analysis.evidence} />
      <Findings title={m.analysis.sections.risks} findings={result.risks} evidence={analysis.evidence} />
      <Findings title={m.analysis.sections.security} findings={result.security} evidence={analysis.evidence} />
      <Findings title={m.analysis.sections.regressions} findings={result.regressions} evidence={analysis.evidence} />
      <Findings title={m.analysis.sections.recommendations} findings={result.recommendations} evidence={analysis.evidence} />
      {analysis.dropped_findings > 0 ? (
        <p className="text-xs text-warning">{formatMessage(m.analysis.dropped, { count: analysis.dropped_findings })}</p>
      ) : null}
    </div>
  );
};
