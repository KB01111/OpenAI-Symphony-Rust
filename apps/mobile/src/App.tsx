import { useMemo, useState } from "react";
import { QueryClient, QueryClientProvider, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Input,
  Field,
  Dialog,
  DialogSurface,
  DialogBody,
  DialogTitle,
  DialogContent,
  DialogActions,
} from "@fluentui/react-components";
import { Button as OaiButton } from "@openai/apps-sdk-ui/components/Button";
import { Badge as OaiBadge } from "@openai/apps-sdk-ui/components/Badge";
import { Alert as OaiAlert } from "@openai/apps-sdk-ui/components/Alert";
import {
  CheckCircle2,
  Cpu,
  KanbanSquare,
  LayoutDashboard,
  Lock,
  Plus,
  RefreshCw,
  ShieldAlert,
  Smartphone,
  Square,
  Play,
  Bell,
} from "lucide-react";

import { SymphonyApiClient } from "@symphony/api-client";
import type { ApprovalDecision } from "@symphony/shared-types";
import {
  SymphonyDesignProvider,
  ProviderBadge,
  TaskStatusBadge,
  PhaseBadge,
  AiApprovalBanner,
  AiEventStreamCard,
  AiEmptyView,
  PromptComposerBox,
} from "@symphony/ui";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});

type MobileTab = "dashboard" | "tasks" | "agents" | "approvals" | "pairing";

export function MobileCompanionShell() {
  const qc = useQueryClient();
  const [activeTab, setActiveTab] = useState<MobileTab>("dashboard");
  const [remoteHostUrl, setRemoteHostUrl] = useState<string>(
    () => localStorage.getItem("symphony.mobile.host") ?? "https://127.0.0.1:47821",
  );
  const [bearerToken, setBearerToken] = useState<string>(
    () => localStorage.getItem("symphony.mobile.token") ?? "",
  );
  const [pushSubscribed, setPushSubscribed] = useState<boolean>(true);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [confirmCancelTaskId, setConfirmCancelTaskId] = useState<string | null>(null);
  const [newTaskTitle, setNewTaskTitle] = useState<string>("");
  const [newTaskDesc, setNewTaskDesc] = useState<string>("");

  const apiClient = useMemo(
    () =>
      new SymphonyApiClient({
        baseUrl: remoteHostUrl,
        bearerToken: bearerToken || undefined,
      }),
    [remoteHostUrl, bearerToken],
  );

  const stateQuery = useQuery({
    queryKey: ["mobile-state", remoteHostUrl, bearerToken],
    queryFn: () => apiClient.getState(),
    refetchInterval: 4000,
  });

  const tasksQuery = useQuery({
    queryKey: ["mobile-tasks", remoteHostUrl, bearerToken],
    queryFn: () => apiClient.listTasks(),
    refetchInterval: 5000,
  });

  const approvalsQuery = useQuery({
    queryKey: ["mobile-approvals", remoteHostUrl, bearerToken],
    queryFn: () => apiClient.listApprovals(true),
    refetchInterval: 3500,
  });

  const projectsQuery = useQuery({
    queryKey: ["mobile-projects", remoteHostUrl, bearerToken],
    queryFn: () => apiClient.listProjects(),
  });

  const activeTaskId = selectedTaskId ?? tasksQuery.data?.[0]?.id ?? null;

  const eventsQuery = useQuery({
    queryKey: ["mobile-events", activeTaskId, remoteHostUrl],
    queryFn: () => (activeTaskId ? apiClient.listTaskEvents(activeTaskId, 50) : Promise.resolve([])),
    enabled: Boolean(activeTaskId),
    refetchInterval: 3000,
  });

  const resolveApprovalMutation = useMutation({
    mutationFn: ({ id, decision }: { id: string; decision: ApprovalDecision }) =>
      apiClient.resolveApproval(id, decision),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["mobile-approvals"] });
    },
  });

  const cancelTaskMutation = useMutation({
    mutationFn: (id: string) => apiClient.cancelTask(id),
    onSuccess: () => {
      setConfirmCancelTaskId(null);
      void qc.invalidateQueries({ queryKey: ["mobile-tasks"] });
      void qc.invalidateQueries({ queryKey: ["mobile-state"] });
    },
  });

  const dispatchTaskMutation = useMutation({
    mutationFn: (id: string) => apiClient.dispatchTaskNow(id),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["mobile-tasks"] });
      void qc.invalidateQueries({ queryKey: ["mobile-state"] });
    },
  });

  const createTaskMutation = useMutation({
    mutationFn: () => {
      const defaultProject = projectsQuery.data?.[0];
      if (!defaultProject) {
        throw new Error("No registered project on remote Symphony host");
      }
      return apiClient.createManualTask({
        project_id: defaultProject.id,
        title: newTaskTitle.trim() || "Mobile Companion Task",
        description: newTaskDesc,
        priority: 2,
      });
    },
    onSuccess: () => {
      setNewTaskTitle("");
      setNewTaskDesc("");
      void qc.invalidateQueries({ queryKey: ["mobile-tasks"] });
    },
  });

  const snapshot = stateQuery.data;
  const tasks = tasksQuery.data ?? [];
  const approvals = approvalsQuery.data ?? [];

  return (
    <SymphonyDesignProvider themeMode="dark">
      <div className="flex flex-col h-screen w-screen bg-slate-950 text-slate-100 overflow-hidden select-none">
        {/* Top Mobile Status Header */}
        <header className="px-4 py-3 border-b border-slate-800/80 bg-slate-900/90 flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <div className="w-7 h-7 rounded-lg bg-blue-600/20 border border-blue-500/40 flex items-center justify-center text-blue-400">
              <Smartphone className="w-4 h-4" />
            </div>
            <div>
              <div className="text-sm font-semibold leading-tight">Symphony Companion</div>
              <div className="text-[11px] text-slate-400 font-mono truncate max-w-[190px]">
                {remoteHostUrl}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <OaiBadge color={snapshot ? "success" : "warning"} variant="soft" size="sm" pill>
              {snapshot ? `${snapshot.counts.running} Active` : "Connecting"}
            </OaiBadge>
            <OaiButton
              color="secondary"
              variant="ghost"
              size="xs"
              uniform
              onClick={() => void stateQuery.refetch()}
            >
              <RefreshCw className="w-3.5 h-3.5" />
            </OaiButton>
          </div>
        </header>

        {/* Main Scrollable Content */}
        <main className="flex-1 overflow-y-auto p-4 space-y-4">
          {activeTab === "dashboard" && (
            <>
              <div className="grid grid-cols-2 gap-3">
                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-3.5">
                  <div className="text-xs text-slate-400">Active Agents</div>
                  <div className="text-2xl font-semibold text-blue-400 mt-1">
                    {snapshot?.counts.running ?? 0}
                  </div>
                </div>
                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-3.5">
                  <div className="text-xs text-slate-400">Pending Approvals</div>
                  <div className="text-2xl font-semibold text-amber-400 mt-1">
                    {approvals.length}
                  </div>
                </div>
                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-3.5">
                  <div className="text-xs text-slate-400">Total Tokens</div>
                  <div className="text-lg font-semibold font-mono text-slate-200 mt-1">
                    {(snapshot?.codex_totals.total_tokens ?? 0).toLocaleString()}
                  </div>
                </div>
                <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-3.5">
                  <div className="text-xs text-slate-400">Host Power</div>
                  <div className="text-sm font-semibold text-emerald-400 mt-1.5">
                    {snapshot?.hardware.on_ac_power ? "AC Power" : "Battery"} •{" "}
                    {Math.round(snapshot?.hardware.cpu_usage_percent ?? 0)}% CPU
                  </div>
                </div>
              </div>

              <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-4 flex items-center justify-between">
                <div className="flex items-center gap-2.5">
                  <Bell className="w-4 h-4 text-blue-400" />
                  <div>
                    <div className="text-xs font-semibold">APNs / WebPush Relay</div>
                    <div className="text-[11px] text-slate-400">
                      Instant alerts for tool approvals & PR readiness
                    </div>
                  </div>
                </div>
                <OaiButton
                  color={pushSubscribed ? "success" : "secondary"}
                  variant="soft"
                  size="xs"
                  onClick={() => setPushSubscribed((v) => !v)}
                >
                  {pushSubscribed ? "Subscribed" : "Muted"}
                </OaiButton>
              </div>

              <div className="space-y-2.5">
                <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                  Live Remote Sessions
                </h2>
                {(snapshot?.running ?? []).length === 0 ? (
                  <AiEmptyView
                    title="No active remote agents"
                    description="Start a task from the Tasks tab to launch an isolated session on your Windows host."
                  />
                ) : (
                  (snapshot?.running ?? []).map((s) => (
                    <div
                      key={s.session_id}
                      className="rounded-xl border border-slate-800 bg-slate-900/70 p-3.5 space-y-2"
                    >
                      <div className="flex items-center justify-between">
                        <span className="font-mono text-xs text-blue-400 font-semibold">
                          {s.issue_identifier}
                        </span>
                        <PhaseBadge phase={s.phase} />
                      </div>
                      <div className="flex items-center justify-between text-xs text-slate-400">
                        <ProviderBadge provider={s.provider} />
                        <OaiButton
                          color="danger"
                          variant="soft"
                          size="xs"
                          onClick={() => setConfirmCancelTaskId(s.task_id)}
                        >
                          <Square className="w-3 h-3" /> Remote Cancel
                        </OaiButton>
                      </div>
                    </div>
                  ))
                )}
              </div>
            </>
          )}

          {activeTab === "tasks" && (
            <div className="space-y-4">
              <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-3.5 space-y-2.5">
                <div className="text-xs font-semibold text-slate-200 flex items-center gap-1.5">
                  <Plus className="w-3.5 h-3.5 text-blue-400" /> Quick Remote Task
                </div>
                <Input
                  placeholder="Task title..."
                  value={newTaskTitle}
                  onChange={(_, d) => setNewTaskTitle(d.value)}
                />
                <PromptComposerBox
                  value={newTaskDesc}
                  onChange={setNewTaskDesc}
                  onSubmit={() => createTaskMutation.mutate()}
                  placeholder="Describe task instructions for remote Windows agent..."
                  submitLabel="Create"
                />
              </div>

              <div className="space-y-2.5">
                {tasks.map((task) => (
                  <div
                    key={task.id}
                    className="rounded-xl border border-slate-800 bg-slate-900/70 p-3.5 space-y-2"
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-mono text-xs text-blue-400 font-semibold">
                        {task.issue.identifier}
                      </span>
                      <TaskStatusBadge status={task.status} />
                    </div>
                    <div className="text-sm font-medium text-slate-100">{task.issue.title}</div>
                    <div className="flex items-center justify-between pt-1">
                      <ProviderBadge provider={task.preferred_provider ?? "codex"} />
                      <div className="flex items-center gap-1.5">
                        <OaiButton
                          color="secondary"
                          variant="outline"
                          size="xs"
                          onClick={() => {
                            setSelectedTaskId(task.id);
                            setActiveTab("agents");
                          }}
                        >
                          Logs
                        </OaiButton>
                        {task.status === "running" ? (
                          <OaiButton
                            color="danger"
                            variant="soft"
                            size="xs"
                            onClick={() => setConfirmCancelTaskId(task.id)}
                          >
                            Cancel
                          </OaiButton>
                        ) : (
                          <OaiButton
                            color="primary"
                            variant="solid"
                            size="xs"
                            onClick={() => dispatchTaskMutation.mutate(task.id)}
                          >
                            <Play className="w-3 h-3" /> Run
                          </OaiButton>
                        )}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {activeTab === "agents" && (
            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                  Execution Stream
                </h2>
                <span className="text-xs font-mono text-blue-400">
                  {tasks.find((t) => t.id === activeTaskId)?.issue.identifier ?? "None"}
                </span>
              </div>
              {(eventsQuery.data ?? []).length === 0 ? (
                <AiEmptyView
                  title="No execution logs"
                  description="Select a task to stream real-time agent events from the Windows runtime."
                />
              ) : (
                <div className="space-y-2.5">
                  {(eventsQuery.data ?? []).map((ev) => (
                    <AiEventStreamCard key={ev.id} event={ev} detailed />
                  ))}
                </div>
              )}
            </div>
          )}

          {activeTab === "approvals" && (
            <div className="space-y-3">
              <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                Remote Security Approvals
              </h2>
              {approvals.length === 0 ? (
                <AiEmptyView
                  title="All clear"
                  description="No pending command or file-patch approval requests."
                  icon={<CheckCircle2 className="w-5 h-5 text-emerald-400" />}
                />
              ) : (
                approvals.map((req) => (
                  <AiApprovalBanner
                    key={req.id}
                    request={req}
                    isSubmitting={resolveApprovalMutation.isPending}
                    onDecide={(id, decision) => resolveApprovalMutation.mutate({ id, decision })}
                  />
                ))
              )}
            </div>
          )}

          {activeTab === "pairing" && (
            <div className="space-y-4">
              <OaiAlert
                color="info"
                variant="soft"
                title="Secure Remote Pairing"
                description="The iOS companion connects directly to the independent Windows Symphony background runtime over TLS with a scoped Bearer token."
              />
              <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4 space-y-3">
                <Field label="Remote Windows Runtime Endpoint (TLS)">
                  <Input
                    value={remoteHostUrl}
                    onChange={(_, d) => {
                      setRemoteHostUrl(d.value);
                      localStorage.setItem("symphony.mobile.host", d.value);
                    }}
                  />
                </Field>
                <Field label="Scoped Pairing Token">
                  <Input
                    type="password"
                    value={bearerToken}
                    onChange={(_, d) => {
                      setBearerToken(d.value);
                      localStorage.setItem("symphony.mobile.token", d.value);
                    }}
                  />
                </Field>
              </div>
            </div>
          )}
        </main>

        {/* Sensitive Remote Action Confirmation Modal */}
        <Dialog
          open={Boolean(confirmCancelTaskId)}
          onOpenChange={(_, data) => {
            if (!data.open) setConfirmCancelTaskId(null);
          }}
        >
          <DialogSurface>
            <DialogBody>
              <DialogTitle>Confirm Remote Cancellation</DialogTitle>
              <DialogContent>
                Are you sure you want to remotely terminate this running agent session on the Windows host?
              </DialogContent>
              <DialogActions>
                <OaiButton
                  color="secondary"
                  variant="outline"
                  size="sm"
                  onClick={() => setConfirmCancelTaskId(null)}
                >
                  Keep Running
                </OaiButton>
                <OaiButton
                  color="danger"
                  variant="solid"
                  size="sm"
                  onClick={() =>
                    confirmCancelTaskId && cancelTaskMutation.mutate(confirmCancelTaskId)
                  }
                >
                  Confirm Remote Cancel
                </OaiButton>
              </DialogActions>
            </DialogBody>
          </DialogSurface>
        </Dialog>

        {/* Bottom iOS Tab Bar */}
        <nav className="border-t border-slate-800 bg-slate-900/95 px-2 py-2 grid grid-cols-5 gap-1">
          {(
            [
              { id: "dashboard", label: "Home", icon: <LayoutDashboard className="w-4 h-4" /> },
              { id: "tasks", label: "Tasks", icon: <KanbanSquare className="w-4 h-4" /> },
              { id: "agents", label: "Stream", icon: <Cpu className="w-4 h-4" /> },
              {
                id: "approvals",
                label: `Approve${approvals.length > 0 ? ` (${approvals.length})` : ""}`,
                icon: <ShieldAlert className="w-4 h-4" />,
              },
              { id: "pairing", label: "Pairing", icon: <Lock className="w-4 h-4" /> },
            ] as const
          ).map((item) => (
            <button
              key={item.id}
              type="button"
              onClick={() => setActiveTab(item.id)}
              className={`flex flex-col items-center justify-center py-1.5 rounded-lg text-[11px] font-medium transition-colors ${
                activeTab === item.id
                  ? "text-blue-400 bg-blue-500/10"
                  : "text-slate-400 hover:text-slate-200"
              }`}
            >
              {item.icon}
              <span className="mt-1">{item.label}</span>
            </button>
          ))}
        </nav>
      </div>
    </SymphonyDesignProvider>
  );
}

export default function MobileApp() {
  return (
    <QueryClientProvider client={queryClient}>
      <MobileCompanionShell />
    </QueryClientProvider>
  );
}
