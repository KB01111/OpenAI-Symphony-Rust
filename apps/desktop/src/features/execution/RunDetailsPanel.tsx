import React, { useState } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@symphony/ui";
import type { Project, ProviderStatus, Task } from "@symphony/shared-types";
import { useMessages } from "../../lib/i18n.js";
import { workbenchMessages } from "../../layouts/messages.js";
import { PullRequestTab, TaskDiffView, UsageTab, ValidationTab } from "./ChangesPanel.js";
import { ContextPanel } from "./ContextPanel.js";
import { executionMessages } from "./messages.js";
import { ToolCallsTab } from "./ToolCallsTab.js";

type PanelTab = "changes" | "validation" | "tools" | "usage" | "pr" | "context";

/** Side panel of a thread: diff, validation, tool calls, usage, pull request and run context. */
export const RunDetailsPanel: React.FC<{
  task: Task;
  project?: Project;
  providerStatus?: ProviderStatus;
  liveSince?: string | null;
}> = ({ task, project, providerStatus, liveSince }) => {
  const m = useMessages(executionMessages);
  const w = useMessages(workbenchMessages).thread;
  const [tab, setTab] = useState<PanelTab>("changes");
  const body = "h-full overflow-auto p-3";
  return (
    <Tabs value={tab} onValueChange={(v) => setTab(v as PanelTab)} className="h-full min-h-0" aria-label={w.panel}>
      <TabsList aria-label={m.changes.tabs}>
        <TabsTrigger value="changes">{m.changes.changes}</TabsTrigger>
        <TabsTrigger value="validation">{m.changes.validation}</TabsTrigger>
        <TabsTrigger value="tools">{m.toolCalls.tab}</TabsTrigger>
        <TabsTrigger value="usage">{m.changes.usage}</TabsTrigger>
        <TabsTrigger value="pr" aria-label={m.changes.pullRequest}>
          {m.changes.pullRequestShort}
        </TabsTrigger>
        <TabsTrigger value="context">{w.tabContext}</TabsTrigger>
      </TabsList>
      <TabsContent value="changes" className={body}>
        <TaskDiffView taskId={task.id} />
      </TabsContent>
      <TabsContent value="validation" className={body}>
        <ValidationTab taskId={task.id} />
      </TabsContent>
      <TabsContent value="tools" className={body}>
        <ToolCallsTab taskId={task.id} />
      </TabsContent>
      <TabsContent value="usage" className={body}>
        <UsageTab task={task} liveSince={liveSince} />
      </TabsContent>
      <TabsContent value="pr" className={body}>
        <PullRequestTab task={task} />
      </TabsContent>
      <TabsContent value="context" className="h-full overflow-auto">
        <ContextPanel task={task} project={project} providerStatus={providerStatus} />
      </TabsContent>
    </Tabs>
  );
};
