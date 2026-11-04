import React, { useEffect } from "react";
import { RefreshCw } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import {
  AiEmptyView,
  Badge,
  Button,
  Hint,
  LoadingRows,
  PageHeader,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger
} from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useKnowledgeOverview, useProjects, useStartKnowledgeIndex } from "../../lib/queries.js";
import { useEventStore } from "../../runtime/eventStore.js";
import { useUiStore } from "../../store/uiStore.js";
import { CommitStrip } from "./CommitStrip.js";
import { CommitsTab } from "./CommitsTab.js";
import { GraphTab } from "./GraphTab.js";
import { InsightsTab } from "./InsightsTab.js";
import { OverviewTab } from "./OverviewTab.js";
import { WorktreesTab } from "./WorktreesTab.js";
import { knowledgeMessages } from "./messages.js";
import { KNOWLEDGE_TABS, KnowledgeSelectionProvider, useKnowledgeSelection, type KnowledgeTab } from "./selection.js";

/** The phase of the index run in progress, from the live `knowledge/index_progress` events. */
function useIndexPhase(projectId: string | null): string | null {
  return useEventStore((state) => {
    for (let i = state.events.length - 1; i >= 0; i--) {
      const event = state.events[i]!;
      if (!event.event_type.startsWith("knowledge/index_")) continue;
      const payload = event.payload as { project_id?: string; phase?: string } | null;
      if (payload?.project_id !== projectId) continue;
      return event.event_type === "knowledge/index_progress" ? (payload.phase ?? null) : null;
    }
    return null;
  });
}

const IndexStatus: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const { projectId } = useKnowledgeSelection();
  const overview = useKnowledgeOverview(projectId);
  const data = overview.data;
  if (!data) return null;
  const running = data.active_run?.status === "running";
  if (running) return <Badge tone="info"><Spinner className="size-3" />{m.status.indexing}</Badge>;
  if (data.active_run?.status === "failed") return <Badge tone="danger">{m.status.failed}</Badge>;
  if (!data.index) return <Badge tone="neutral">{m.status.notIndexed}</Badge>;
  return (
    <Hint label={formatMessage(m.status.indexedTitle, { sha: data.index.commit_sha.slice(0, 8), ref: data.index.ref_name })}>
      <Badge tone={data.index_stale ? "warning" : "success"}>{data.index_stale ? m.status.stale : m.status.indexed}</Badge>
    </Hint>
  );
};

const SyncButton: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const { projectId } = useKnowledgeSelection();
  const overview = useKnowledgeOverview(projectId);
  const start = useStartKnowledgeIndex(projectId);
  const phase = useIndexPhase(projectId);
  const running = overview.data?.active_run?.status === "running" || start.isPending;
  const engineReady = overview.data?.engines.some((engine) => engine.id === "graphify" && engine.available) ?? false;
  const phaseLabel = phase && phase in m.sync.phases ? m.sync.phases[phase as keyof typeof m.sync.phases] : m.sync.indexing;
  const button = (
    <Button size="sm" variant="secondary" disabled={running || !engineReady || !projectId} onClick={() => start.mutate(null)}>
      {running ? <Spinner /> : <RefreshCw className="size-4" />}
      {running ? phaseLabel : m.sync.action}
    </Button>
  );
  // A disabled button shows no tooltip of its own, so the reason sits on a wrapper.
  return !engineReady && overview.data ? (
    <Hint label={m.sync.noEngine}>
      <span tabIndex={0}>{button}</span>
    </Hint>
  ) : (
    button
  );
};

const Content: React.FC<{ projects: ReadonlyArray<{ id: string; name: string }> }> = ({ projects }) => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  const overview = useKnowledgeOverview(selection.projectId);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title={m.title}
        description={
          overview.data ? formatMessage(m.description, { worktrees: overview.data.worktrees_total, dirty: overview.data.worktrees_dirty }) : undefined
        }
        meta={<IndexStatus />}
        actions={
          <>
            <Select value={selection.projectId ?? undefined} onValueChange={(id) => setActiveProject(id)}>
              <SelectTrigger className="w-52" aria-label={m.repository}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {projects.map((project) => (
                  <SelectItem key={project.id} value={project.id}>
                    {project.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <SyncButton />
          </>
        }
      />
      <Tabs value={selection.tab} onValueChange={(value) => selection.setTab(value as KnowledgeTab)} className="min-h-0 flex-1">
        <TabsList aria-label={m.tabsLabel}>
          {KNOWLEDGE_TABS.map((tab) => (
            <TabsTrigger key={tab} value={tab}>
              {m.tabs[tab]}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value="overview" className="overflow-hidden">
          <OverviewTab />
        </TabsContent>
        <TabsContent value="graph" className="overflow-hidden">
          <GraphTab />
        </TabsContent>
        <TabsContent value="commits" className="overflow-hidden">
          <CommitsTab />
        </TabsContent>
        <TabsContent value="worktrees" className="overflow-hidden">
          <WorktreesTab />
        </TabsContent>
        <TabsContent value="insights" className="overflow-hidden">
          <InsightsTab />
        </TabsContent>
      </Tabs>
      <CommitStrip />
    </div>
  );
};

/** Repository Knowledge Base: code graph, live worktrees, commit timeline, impact and AI analysis. */
const KnowledgePage: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const projects = useProjects();
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);
  const list = projects.data ?? [];
  // The app-wide active project when it exists, else the first repository.
  const projectId = list.find((p) => p.id === activeProjectId)?.id ?? list[0]?.id ?? null;

  // Settle on the fallback only once the list is current: right after a repository is added the
  // list is refetched, and its stale copy would overwrite the new selection with the first entry.
  useEffect(() => {
    if (projectId && projectId !== activeProjectId && !projects.isFetching) setActiveProject(projectId);
  }, [projectId, activeProjectId, projects.isFetching, setActiveProject]);

  if (projects.isPending) return <LoadingRows className="p-4" rows={6} />;
  if (list.length === 0) {
    return (
      <div className="p-4">
        <AiEmptyView
          title={m.noProject.title}
          description={m.noProject.description}
          action={
            <Button size="sm" onClick={() => setAddProjectOpen(true)}>
              {m.noProject.action}
            </Button>
          }
        />
      </div>
    );
  }
  return (
    <KnowledgeSelectionProvider projectId={projectId}>
      <Content projects={list} />
    </KnowledgeSelectionProvider>
  );
};

export default KnowledgePage;
