import React, { useEffect, useMemo, useState } from "react";
import {
  useMutation,
  useQuery,
  useQueryClient
} from "@tanstack/react-query";
import {
  Button as FluentButton,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Input,
  ProgressBar,
  Switch
} from "@fluentui/react-components";
import {
  ALL_TASK_STATUSES,
  canTransitionTaskStatus,
  getTranslations,
  type AgentEvent,
  type ApprovalDecision,
  type AppSettings,
  type Locale,
  type ProviderKind,
  type Task,
  type TaskStatus
} from "@symphony/shared-types";
import { defaultSymphonyClient } from "@symphony/api-client";
import {
  AiApprovalBanner,
  AiEmptyView,
  AiEventStreamCard,
  HardwareTelemetryStrip,
  LogModeSwitch,
  MergeReadinessCard,
  PhaseBadge,
  PromptComposerBox,
  ProviderBadge,
  SdkAlert,
  SdkBadge,
  SdkButton,
  SdkCodeBlock,
  SdkMarkdown,
  SdkSegmentedControl,
  SdkTooltip,
  SymphonyDesignProvider,
  TaskStatusBadge
} from "@symphony/ui";
import { useUiStore, type DesktopSection } from "./store/uiStore.js";
import {
  Activity,
  Bot,
  Command,
  FileCode,
  FolderGit2,
  GitPullRequest,
  Kanban,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings as SettingsIcon,
  Shield,
  Sliders,
  Sparkles,
  Terminal,
  Trash2
} from "lucide-react";

export const App: React.FC = () => {
  const queryClient = useQueryClient();
  const {
    activeSection,
    selectedProjectId,
    selectedTaskId,
    selectedIssueIdentifier,
    logMode,
    themeMode,
    locale,
    commandPaletteOpen,
    onboardingOpen,
    setActiveSection,
    selectProject,
    selectTask,
    setLogMode,
    setThemeMode,
    setLocale,
    setCommandPaletteOpen,
    setOnboardingOpen
  } = useUiStore();

  const t = useMemo(() => getTranslations(locale), [locale]);

  const [liveEvents, setLiveEvents] = useState<AgentEvent[]>([]);
  const [newTaskDialogOpen, setNewTaskDialogOpen] = useState(false);
  const [newProjectDialogOpen, setNewProjectDialogOpen] = useState(false);
  const [commandSearch, setCommandSearch] = useState("");
  const [diagnosticJson, setDiagnosticJson] = useState<string | null>(null);

  // New Task Form State
  const [taskTitle, setTaskTitle] = useState("");
  const [taskDescription, setTaskDescription] = useState("");
  const [taskIdentifier, setTaskIdentifier] = useState("");
  const [taskPriority, setTaskPriority] = useState<number>(2);
  const [taskProvider, setTaskProvider] = useState<ProviderKind>("codex");
  const [taskDispatchNow, setTaskDispatchNow] = useState(true);

  // New Project Form State
  const [projName, setProjName] = useState("");
  const [projRepoPath, setProjRepoPath] = useState("");
  const [projDefaultBranch, setProjDefaultBranch] = useState("main");
  const [projWorkspaceRoot] = useState("");
  const [projProvider, setProjProvider] = useState<ProviderKind>("codex");
  const [projTrackerKind, setProjTrackerKind] = useState("github");
  const [projGithubRepo, setProjGithubRepo] = useState("");
  const [projLinearSlug] = useState("");
  const [projLabels] = useState("symphony, agent-ready");
  const [projValidationCmds, setProjValidationCmds] = useState("cargo test --workspace");
  const [projAutoDispatch, setProjAutoDispatch] = useState(false);

  // Workflow Editor State
  const [workflowDraft, setWorkflowDraft] = useState<string | null>(null);

  // Credential Vault Input State
  const [credKey, setCredKey] = useState("GITHUB_TOKEN");
  const [credValue, setCredValue] = useState("");

  // Queries
  const stateQuery = useQuery({
    queryKey: ["orchestrator-state"],
    queryFn: () => defaultSymphonyClient.getState(),
    refetchInterval: 2500
  });

  const projectsQuery = useQuery({
    queryKey: ["projects"],
    queryFn: () => defaultSymphonyClient.listProjects()
  });

  const tasksQuery = useQuery({
    queryKey: ["tasks", selectedProjectId],
    queryFn: () => defaultSymphonyClient.listTasks(selectedProjectId ?? undefined),
    refetchInterval: 2500
  });

  const approvalsQuery = useQuery({
    queryKey: ["approvals"],
    queryFn: () => defaultSymphonyClient.listApprovals(true),
    refetchInterval: 2000
  });

  const prReviewsQuery = useQuery({
    queryKey: ["prs", selectedProjectId],
    queryFn: () => defaultSymphonyClient.listPrReviews(selectedProjectId ?? undefined),
    refetchInterval: 4000
  });

  const providersQuery = useQuery({
    queryKey: ["providers"],
    queryFn: () => defaultSymphonyClient.listProviders()
  });

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => defaultSymphonyClient.getSettings()
  });

  const workflowQuery = useQuery({
    queryKey: ["workflow"],
    queryFn: () => defaultSymphonyClient.getWorkflow()
  });

  const activeTask: Task | null = useMemo(() => {
    const list = tasksQuery.data ?? [];
    if (selectedTaskId) {
      const found = list.find((item) => item.id === selectedTaskId);
      if (found) return found;
    }
    return list[0] ?? null;
  }, [tasksQuery.data, selectedTaskId]);

  const activeIdentifier =
    selectedIssueIdentifier ?? activeTask?.issue.identifier ?? null;

  const issueDetailQuery = useQuery({
    queryKey: ["issue-detail", activeIdentifier],
    queryFn: () =>
      activeIdentifier
        ? defaultSymphonyClient.getIssueDetail(activeIdentifier)
        : Promise.resolve(null),
    enabled: Boolean(activeIdentifier),
    refetchInterval: 2000
  });

  // Synchronize initial settings
  useEffect(() => {
    if (settingsQuery.data) {
      setThemeMode(settingsQuery.data.theme);
      setLocale(settingsQuery.data.language);
      if (!settingsQuery.data.onboarding_completed) {
        setOnboardingOpen(true);
      }
    }
  }, [settingsQuery.data, setThemeMode, setLocale, setOnboardingOpen]);

  useEffect(() => {
    if (workflowQuery.data && workflowDraft === null) {
      setWorkflowDraft(workflowQuery.data.raw_content);
    }
  }, [workflowQuery.data, workflowDraft]);

  // Subscribe to SSE event stream with batched updates
  useEffect(() => {
    const unsubscribe = defaultSymphonyClient.subscribeEvents((batch) => {
      setLiveEvents((prev) => [...batch.reverse(), ...prev].slice(0, 250));
      void queryClient.invalidateQueries({ queryKey: ["orchestrator-state"] });
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
    });
    return unsubscribe;
  }, [queryClient]);

  // Keyboard shortcuts (Ctrl+K command palette, Ctrl+N new task, Ctrl+R refresh)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCommandPaletteOpen(!commandPaletteOpen);
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setNewTaskDialogOpen(true);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [commandPaletteOpen, setCommandPaletteOpen]);

  // Mutations
  const refreshMutation = useMutation({
    mutationFn: () => defaultSymphonyClient.triggerRefresh(),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["orchestrator-state"] });
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
    }
  });

  const pauseSchedulerMutation = useMutation({
    mutationFn: (paused: boolean) => defaultSymphonyClient.setSchedulerPaused(paused),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["orchestrator-state"] });
    }
  });

  const createProjectMutation = useMutation({
    mutationFn: () =>
      defaultSymphonyClient.createProject({
        name: projName,
        repo_path: projRepoPath,
        default_branch: projDefaultBranch,
        workspace_root: projWorkspaceRoot || undefined,
        preferred_provider: projProvider,
        tracker_kind: projTrackerKind,
        github_repo: projGithubRepo || null,
        linear_project_slug: projLinearSlug || null,
        required_labels: projLabels
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
        auto_dispatch: projAutoDispatch,
        validation_commands: projValidationCmds
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean)
      }),
    onSuccess: (created) => {
      setNewProjectDialogOpen(false);
      setProjName("");
      selectProject(created.id);
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    }
  });

  const deleteProjectMutation = useMutation({
    mutationFn: (id: string) => defaultSymphonyClient.deleteProject(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    }
  });

  const createTaskMutation = useMutation({
    mutationFn: () => {
      const targetProjectId =
        selectedProjectId ?? projectsQuery.data?.[0]?.id ?? "default";
      return defaultSymphonyClient.createManualTask({
        project_id: targetProjectId,
        identifier: taskIdentifier.trim() ? taskIdentifier.trim() : null,
        title: taskTitle.trim(),
        description: taskDescription.trim() || null,
        priority: taskPriority,
        provider: taskProvider,
        labels: ["symphony"],
        dispatch_immediately: taskDispatchNow
      });
    },
    onSuccess: (created) => {
      setNewTaskDialogOpen(false);
      setTaskTitle("");
      setTaskDescription("");
      setTaskIdentifier("");
      selectTask(created.id, created.issue.identifier);
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["orchestrator-state"] });
    }
  });

  const statusTransitionMutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: TaskStatus }) =>
      defaultSymphonyClient.updateTaskStatus(id, status),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["orchestrator-state"] });
    }
  });

  const startTaskMutation = useMutation({
    mutationFn: (id: string) => defaultSymphonyClient.startTask(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["orchestrator-state"] });
    }
  });

  const cancelTaskMutation = useMutation({
    mutationFn: (id: string) => defaultSymphonyClient.cancelTask(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["orchestrator-state"] });
      void queryClient.invalidateQueries({ queryKey: ["issue-detail"] });
    }
  });

  const retryTaskMutation = useMutation({
    mutationFn: (id: string) => defaultSymphonyClient.retryTask(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["orchestrator-state"] });
      void queryClient.invalidateQueries({ queryKey: ["issue-detail"] });
    }
  });

  const resolveApprovalMutation = useMutation({
    mutationFn: ({
      id,
      decision
    }: {
      id: string;
      decision: ApprovalDecision;
    }) => defaultSymphonyClient.resolveApproval(id, decision),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["approvals"] });
      void queryClient.invalidateQueries({ queryKey: ["issue-detail"] });
    }
  });

  const approvePrMutation = useMutation({
    mutationFn: (id: string) => defaultSymphonyClient.approvePrMerge(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["prs"] });
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["issue-detail"] });
    }
  });

  const saveWorkflowMutation = useMutation({
    mutationFn: (raw: string) => defaultSymphonyClient.saveWorkflow(raw),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["workflow"] });
    }
  });

  const saveSettingsMutation = useMutation({
    mutationFn: (next: AppSettings) => defaultSymphonyClient.saveSettings(next),
    onSuccess: (saved) => {
      setThemeMode(saved.theme);
      setLocale(saved.language);
      void queryClient.invalidateQueries({ queryKey: ["settings"] });
    }
  });

  const saveCredentialMutation = useMutation({
    mutationFn: () => defaultSymphonyClient.saveCredential(credKey, credValue),
    onSuccess: () => {
      setCredValue("");
    }
  });

  const snapshot = stateQuery.data;
  const tasks = tasksQuery.data ?? [];
  const projects = projectsQuery.data ?? [];
  const approvals = approvalsQuery.data ?? [];
  const prReviews = prReviewsQuery.data ?? [];
  const providers = providersQuery.data ?? [];
  const combinedEvents = useMemo(() => {
    const map = new Map<string, AgentEvent>();
    for (const ev of [...liveEvents, ...(snapshot?.recent_events ?? [])]) {
      map.set(ev.id, ev);
    }
    return Array.from(map.values()).slice(0, 120);
  }, [liveEvents, snapshot?.recent_events]);

  const navItems: {
    id: DesktopSection;
    label: string;
    icon: React.ReactNode;
    badge?: number;
  }[] = [
    {
      id: "dashboard",
      label: t.nav.dashboard,
      icon: <Activity className="w-4 h-4" />
    },
    {
      id: "projects",
      label: t.nav.projects,
      icon: <FolderGit2 className="w-4 h-4" />,
      badge: projects.length
    },
    {
      id: "taskBoard",
      label: t.nav.taskBoard,
      icon: <Kanban className="w-4 h-4" />,
      badge: tasks.length
    },
    {
      id: "agentControl",
      label: t.nav.agentControl,
      icon: <Bot className="w-4 h-4" />,
      badge: snapshot?.counts.running ?? 0
    },
    {
      id: "executionView",
      label: t.nav.executionView,
      icon: <Terminal className="w-4 h-4" />,
      badge: approvals.length > 0 ? approvals.length : undefined
    },
    {
      id: "prReview",
      label: t.nav.prReview,
      icon: <GitPullRequest className="w-4 h-4" />,
      badge: prReviews.filter((r) => !r.human_approved).length || undefined
    },
    {
      id: "workflowEditor",
      label: t.nav.workflowEditor,
      icon: <FileCode className="w-4 h-4" />
    },
    {
      id: "settings",
      label: t.nav.settings,
      icon: <SettingsIcon className="w-4 h-4" />
    }
  ];

  return (
    <SymphonyDesignProvider themeMode={themeMode}>
      <div className="flex h-screen w-screen overflow-hidden bg-[#0d1117] text-slate-100">
        {/* Left Windows 11 Fluent Navigation Rail */}
        <aside className="w-64 shrink-0 border-r border-slate-800/80 bg-[#11161f] flex flex-col justify-between select-none">
          <div>
            {/* Brand Header */}
            <div className="px-4 py-4 border-b border-slate-800/80 flex items-center justify-between">
              <div className="flex items-center gap-2.5">
                <div className="w-8 h-8 rounded-lg bg-blue-600/20 border border-blue-500/40 flex items-center justify-center">
                  <Sparkles className="w-4 h-4 text-blue-400" />
                </div>
                <div>
                  <div className="text-sm font-semibold tracking-tight">
                    {t.appName}
                  </div>
                  <div className="text-[11px] text-slate-400">
                    OpenAI Symphony • Rust 1.0
                  </div>
                </div>
              </div>
              <SdkBadge variant="outline" color="info" size="sm">
                v1.0
              </SdkBadge>
            </div>

            {/* Project Filter Selector */}
            <div className="px-3 py-2.5 border-b border-slate-800/60">
              <label className="block text-[11px] uppercase tracking-wider text-slate-400 mb-1">
                {t.tasks.project}
              </label>
              <select
                aria-label="Active project filter"
                className="w-full rounded-md border border-slate-700/70 bg-slate-900/90 px-2.5 py-1.5 text-xs text-slate-100"
                value={selectedProjectId ?? ""}
                onChange={(e) =>
                  selectProject(e.target.value ? e.target.value : null)
                }
              >
                <option value="">All Registered Projects ({projects.length})</option>
                {projects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name} ({p.preferred_provider})
                  </option>
                ))}
              </select>
            </div>

            {/* Navigation Items */}
            <nav className="p-2 space-y-1" aria-label="Main Navigation">
              {navItems.map((item) => {
                const isActive = activeSection === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setActiveSection(item.id)}
                    className={`w-full flex items-center justify-between px-3 py-2 rounded-lg text-xs font-medium transition-colors ${
                      isActive
                        ? "bg-blue-600/20 text-blue-300 border border-blue-500/30"
                        : "text-slate-300 hover:bg-slate-800/60 hover:text-white"
                    }`}
                  >
                    <span className="flex items-center gap-2.5">
                      {item.icon}
                      <span>{item.label}</span>
                    </span>
                    {typeof item.badge === "number" && item.badge > 0 ? (
                      <SdkBadge
                        variant="soft"
                        color={isActive ? "info" : "secondary"}
                        size="sm"
                        pill
                      >
                        {item.badge}
                      </SdkBadge>
                    ) : null}
                  </button>
                );
              })}
            </nav>
          </div>

          {/* Bottom Quick Controls & Language/Command Bar */}
          <div className="p-3 border-t border-slate-800/80 space-y-2.5 bg-slate-950/40">
            <SdkButton
              color="primary"
              variant="solid"
              size="sm"
              block
              onClick={() => setNewTaskDialogOpen(true)}
            >
              <Plus className="w-3.5 h-3.5 mr-1" />
              {t.tasks.newTask}
            </SdkButton>

            <div className="flex items-center justify-between gap-2">
              <button
                type="button"
                onClick={() => setCommandPaletteOpen(true)}
                className="flex-1 flex items-center justify-between rounded-md border border-slate-700/70 bg-slate-900/70 px-2.5 py-1 text-[11px] text-slate-400 hover:text-slate-200"
              >
                <span className="flex items-center gap-1">
                  <Command className="w-3 h-3" />
                  Command
                </span>
                <kbd className="font-mono text-[10px]">Ctrl+K</kbd>
              </button>

              <SdkSegmentedControl
                aria-label="Interface language"
                value={locale}
                onChange={(val) => setLocale(val as Locale)}
                size="2xs"
              >
                <SdkSegmentedControl.Option value="en">EN</SdkSegmentedControl.Option>
                <SdkSegmentedControl.Option value="sv">SV</SdkSegmentedControl.Option>
              </SdkSegmentedControl>
            </div>
          </div>
        </aside>

        {/* Main Workspace Surface */}
        <div className="flex-1 flex flex-col min-w-0 overflow-hidden">
          {/* Top Windows 11 Command & Telemetry Header */}
          <header className="h-14 shrink-0 border-b border-slate-800/80 bg-[#11161f]/90 px-5 flex items-center justify-between gap-4">
            <div className="flex items-center gap-3 min-w-0">
              <h1 className="text-sm font-semibold truncate">
                {navItems.find((n) => n.id === activeSection)?.label}
              </h1>
              {snapshot?.scheduler_paused ? (
                <SdkBadge variant="solid" color="warning" size="sm" pill>
                  Scheduler Paused
                </SdkBadge>
              ) : (
                <SdkBadge variant="soft" color="success" size="sm" pill>
                  Orchestrator Active
                </SdkBadge>
              )}
              {approvals.length > 0 ? (
                <SdkBadge variant="solid" color="warning" size="sm" pill>
                  {approvals.length} Approval Pending
                </SdkBadge>
              ) : null}
            </div>

            <div className="flex items-center gap-3">
              {snapshot?.hardware ? (
                <HardwareTelemetryStrip
                  cpuPercent={snapshot.hardware.cpu_usage_percent}
                  memoryPercent={snapshot.hardware.memory_usage_percent}
                  availableMemoryMb={snapshot.hardware.available_memory_mb}
                  onAcPower={snapshot.hardware.on_ac_power}
                  batteryPercent={snapshot.hardware.battery_percent}
                  recommendedMaxConcurrency={
                    snapshot.hardware.recommended_max_concurrency
                  }
                />
              ) : null}

              <SdkTooltip content={t.dashboard.refreshNow}>
                <SdkButton
                  color="secondary"
                  variant="outline"
                  size="xs"
                  onClick={() => refreshMutation.mutate()}
                >
                  <RefreshCw className="w-3.5 h-3.5 mr-1" />
                  {t.dashboard.refreshNow}
                </SdkButton>
              </SdkTooltip>

              <SdkButton
                color={snapshot?.scheduler_paused ? "success" : "warning"}
                variant="soft"
                size="xs"
                onClick={() =>
                  pauseSchedulerMutation.mutate(!snapshot?.scheduler_paused)
                }
              >
                {snapshot?.scheduler_paused ? (
                  <>
                    <Play className="w-3.5 h-3.5 mr-1" />
                    {t.dashboard.resumeScheduler}
                  </>
                ) : (
                  <>
                    <Pause className="w-3.5 h-3.5 mr-1" />
                    {t.dashboard.pauseScheduler}
                  </>
                )}
              </SdkButton>
            </div>
          </header>

          {/* Pending Approvals Global Bar */}
          {approvals.length > 0 && activeSection !== "executionView" ? (
            <div className="px-5 pt-3">
              <AiApprovalBanner
                approval={approvals[0]}
                onDecide={(id, decision) =>
                  resolveApprovalMutation.mutate({ id, decision })
                }
              />
            </div>
          ) : null}

          {/* Content Scroll Area */}
          <main className="flex-1 overflow-y-auto p-5 space-y-5">
            {/* SECTION A: DASHBOARD */}
            {activeSection === "dashboard" ? (
              <div className="space-y-5">
                {/* KPI Metric Grid */}
                <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-8 gap-3">
                  {[
                    {
                      label: t.dashboard.activeAgents,
                      value: snapshot?.running.length ?? 0,
                      tone: "text-blue-400"
                    },
                    {
                      label: t.dashboard.runningTasks,
                      value: snapshot?.counts.running ?? 0,
                      tone: "text-sky-400"
                    },
                    {
                      label: t.dashboard.queuedTasks,
                      value: snapshot?.counts.queued ?? 0,
                      tone: "text-indigo-400"
                    },
                    {
                      label: t.dashboard.blockedTasks,
                      value: snapshot?.counts.blocked ?? 0,
                      tone: "text-amber-400"
                    },
                    {
                      label: t.dashboard.completedTasks,
                      value: snapshot?.counts.completed ?? 0,
                      tone: "text-emerald-400"
                    },
                    {
                      label: t.dashboard.failedTasks,
                      value: snapshot?.counts.failed ?? 0,
                      tone: "text-red-400"
                    },
                    {
                      label: t.dashboard.tokenUsage,
                      value: (
                        snapshot?.codex_totals.total_tokens ?? 0
                      ).toLocaleString(),
                      tone: "text-purple-400"
                    },
                    {
                      label: t.dashboard.estimatedCost,
                      value: `$${(
                        snapshot?.codex_totals.estimated_cost_usd ?? 0
                      ).toFixed(3)}`,
                      tone: "text-teal-400"
                    }
                  ].map((metric, idx) => (
                    <div
                      key={idx}
                      className="rounded-xl border border-slate-800/80 bg-[#131922] p-3.5"
                    >
                      <div className="text-[11px] text-slate-400 truncate">
                        {metric.label}
                      </div>
                      <div className={`text-xl font-bold mt-1 ${metric.tone}`}>
                        {metric.value}
                      </div>
                    </div>
                  ))}
                </div>

                {/* Active Sessions & Recent PRs Row */}
                <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                  {/* Active Running Agents */}
                  <div className="rounded-xl border border-slate-800/80 bg-[#131922] p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <h2 className="text-sm font-semibold flex items-center gap-2">
                        <Bot className="w-4 h-4 text-blue-400" />
                        {t.agents.activeSessions}
                      </h2>
                      <SdkBadge variant="soft" color="info" size="sm" pill>
                        {snapshot?.running.length ?? 0} Running
                      </SdkBadge>
                    </div>

                    {(snapshot?.running.length ?? 0) === 0 ? (
                      <AiEmptyView
                        title={t.agents.noActiveAgents}
                        description="Create a development task or enable repository polling to dispatch coding agents in isolated Git worktrees."
                        actionLabel={t.tasks.newTask}
                        onAction={() => setNewTaskDialogOpen(true)}
                      />
                    ) : (
                      <div className="space-y-2.5">
                        {snapshot?.running.map((session) => (
                          <div
                            key={session.session_id}
                            className="rounded-lg border border-slate-700/60 bg-slate-900/60 p-3 flex items-center justify-between gap-3"
                          >
                            <div className="space-y-1 min-w-0">
                              <div className="flex items-center gap-2">
                                <span className="font-mono text-xs font-semibold text-blue-300">
                                  {session.issue_identifier}
                                </span>
                                <ProviderBadge
                                  provider={session.provider}
                                  model={session.model}
                                />
                                <PhaseBadge phase={session.phase} />
                              </div>
                              <div className="text-xs text-slate-300 truncate">
                                {session.current_operation}
                              </div>
                              <div className="text-[11px] text-slate-400 font-mono truncate">
                                Worktree: {session.workspace_path}
                              </div>
                            </div>
                            <div className="flex items-center gap-2 shrink-0">
                              <SdkButton
                                color="secondary"
                                variant="outline"
                                size="xs"
                                onClick={() => {
                                  selectTask(
                                    session.task_id,
                                    session.issue_identifier
                                  );
                                  setActiveSection("executionView");
                                }}
                              >
                                {t.agents.openDetails}
                              </SdkButton>
                              <SdkButton
                                color="danger"
                                variant="ghost"
                                size="xs"
                                onClick={() =>
                                  cancelTaskMutation.mutate(session.task_id)
                                }
                              >
                                {t.agents.cancelExecution}
                              </SdkButton>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>

                  {/* Recent Pull Requests */}
                  <div className="rounded-xl border border-slate-800/80 bg-[#131922] p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <h2 className="text-sm font-semibold flex items-center gap-2">
                        <GitPullRequest className="w-4 h-4 text-emerald-400" />
                        {t.dashboard.recentPullRequests}
                      </h2>
                      <SdkButton
                        color="secondary"
                        variant="ghost"
                        size="2xs"
                        onClick={() => setActiveSection("prReview")}
                      >
                        View All ({prReviews.length})
                      </SdkButton>
                    </div>

                    {prReviews.length === 0 ? (
                      <div className="text-xs text-slate-400 py-8 text-center border border-dashed border-slate-800 rounded-lg">
                        Completed agent runs automatically prepare pull request readiness reports here.
                      </div>
                    ) : (
                      <div className="space-y-2.5">
                        {prReviews.slice(0, 4).map((pr) => (
                          <div
                            key={pr.id}
                            className="rounded-lg border border-slate-700/60 bg-slate-900/60 p-3 flex items-center justify-between gap-3"
                          >
                            <div className="min-w-0">
                              <div className="flex items-center gap-2">
                                <span className="text-xs font-semibold truncate">
                                  PR #{pr.pr_number}: {pr.title}
                                </span>
                                <SdkBadge
                                  variant="solid"
                                  color={
                                    pr.merge_readiness.score >= 4
                                      ? "success"
                                      : "warning"
                                  }
                                  size="sm"
                                  pill
                                >
                                  Readiness {pr.merge_readiness.score}/5
                                </SdkBadge>
                              </div>
                              <div className="text-[11px] text-slate-400 font-mono mt-0.5">
                                {pr.head_branch} → {pr.base_branch} •{" "}
                                {pr.changed_files.length} files
                              </div>
                            </div>
                            <SdkButton
                              color="primary"
                              variant="soft"
                              size="xs"
                              onClick={() => setActiveSection("prReview")}
                            >
                              Review PR
                            </SdkButton>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </div>

                {/* Live Event Feed */}
                <div className="rounded-xl border border-slate-800/80 bg-[#131922] p-4 space-y-3">
                  <div className="flex items-center justify-between">
                    <h2 className="text-sm font-semibold flex items-center gap-2">
                      <Terminal className="w-4 h-4 text-indigo-400" />
                      {t.dashboard.liveEventFeed}
                    </h2>
                    <LogModeSwitch
                      mode={logMode}
                      onChange={setLogMode}
                      conciseLabel={t.agents.conciseMode}
                      detailedLabel={t.agents.detailedLogsMode}
                    />
                  </div>

                  {combinedEvents.length === 0 ? (
                    <div className="text-xs text-slate-400 py-6 text-center">
                      {t.dashboard.noRecentEvents}
                    </div>
                  ) : (
                    <div className="space-y-2 max-h-96 overflow-y-auto pr-1">
                      {combinedEvents.slice(0, 40).map((ev) => (
                        <AiEventStreamCard
                          key={ev.id}
                          event={ev}
                          detailedMode={logMode === "detailed"}
                        />
                      ))}
                    </div>
                  )}
                </div>
              </div>
            ) : null}

            {/* SECTION B: PROJECTS */}
            {activeSection === "projects" ? (
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <div>
                    <h2 className="text-base font-semibold">
                      Registered Repositories & Issue Trackers
                    </h2>
                    <p className="text-xs text-slate-400">
                      Connect local Git repositories, GitHub Issues, or Linear projects with isolated worktree roots.
                    </p>
                  </div>
                  <SdkButton
                    color="primary"
                    variant="solid"
                    size="sm"
                    onClick={() => setNewProjectDialogOpen(true)}
                  >
                    <Plus className="w-4 h-4 mr-1" />
                    Register Project
                  </SdkButton>
                </div>

                {projects.length === 0 ? (
                  <AiEmptyView
                    title="No repositories registered yet"
                    description="Register a local Git repository and connect GitHub Issues, Linear, or manual tasks."
                    actionLabel="Register Project"
                    onAction={() => setNewProjectDialogOpen(true)}
                  />
                ) : (
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    {projects.map((project) => (
                      <div
                        key={project.id}
                        className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-3"
                      >
                        <div className="flex items-start justify-between gap-2">
                          <div>
                            <div className="flex items-center gap-2">
                              <FolderGit2 className="w-4 h-4 text-blue-400" />
                              <h3 className="text-sm font-semibold">
                                {project.name}
                              </h3>
                              <ProviderBadge
                                provider={project.preferred_provider}
                                model={project.preferred_model}
                              />
                            </div>
                            <p className="text-xs font-mono text-slate-400 mt-1">
                              Repo: {project.repo_path} ({project.default_branch})
                            </p>
                            <p className="text-xs font-mono text-slate-400">
                              Worktrees: {project.workspace_root}
                            </p>
                          </div>
                          <SdkButton
                            color="danger"
                            variant="ghost"
                            size="2xs"
                            onClick={() => deleteProjectMutation.mutate(project.id)}
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </SdkButton>
                        </div>

                        <div className="flex flex-wrap gap-1.5 text-xs">
                          <SdkBadge variant="outline" color="info" size="sm">
                            Tracker: {project.tracker_kind}
                          </SdkBadge>
                          {project.github_repo ? (
                            <SdkBadge variant="soft" color="discovery" size="sm">
                              GitHub: {project.github_repo}
                            </SdkBadge>
                          ) : null}
                          {project.linear_project_slug ? (
                            <SdkBadge variant="soft" color="info" size="sm">
                              Linear: {project.linear_project_slug}
                            </SdkBadge>
                          ) : null}
                          <SdkBadge
                            variant="soft"
                            color={project.auto_dispatch ? "success" : "secondary"}
                            size="sm"
                          >
                            {project.auto_dispatch
                              ? "Auto-Dispatch Opt-In Enabled"
                              : "Manual Dispatch Policy"}
                          </SdkBadge>
                        </div>

                        <div className="flex items-center justify-end gap-2 pt-1">
                          <SdkButton
                            color="secondary"
                            variant="outline"
                            size="xs"
                            onClick={() => {
                              selectProject(project.id);
                              setActiveSection("taskBoard");
                            }}
                          >
                            Open Task Board
                          </SdkButton>
                          <SdkButton
                            color="primary"
                            variant="soft"
                            size="xs"
                            onClick={() => {
                              selectProject(project.id);
                              setNewTaskDialogOpen(true);
                            }}
                          >
                            Create Task
                          </SdkButton>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ) : null}

            {/* SECTION C: KANBAN TASK BOARD */}
            {activeSection === "taskBoard" ? (
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <div>
                    <h2 className="text-base font-semibold">{t.nav.taskBoard}</h2>
                    <p className="text-xs text-slate-400">
                      Validated Symphony state machine transitions across all 9 task lifecycle columns.
                    </p>
                  </div>
                  <SdkButton
                    color="primary"
                    variant="solid"
                    size="sm"
                    onClick={() => setNewTaskDialogOpen(true)}
                  >
                    <Plus className="w-4 h-4 mr-1" />
                    {t.tasks.newTask}
                  </SdkButton>
                </div>

                <div className="flex gap-3 overflow-x-auto pb-4">
                  {ALL_TASK_STATUSES.map((statusCol) => {
                    const colTasks = tasks.filter((item) => item.status === statusCol);
                    return (
                      <div
                        key={statusCol}
                        className="w-72 shrink-0 rounded-xl border border-slate-800/90 bg-[#131922] p-3 flex flex-col max-h-[calc(100vh-200px)]"
                      >
                        <div className="flex items-center justify-between mb-2.5 pb-2 border-b border-slate-800">
                          <TaskStatusBadge
                            status={statusCol}
                            label={t.status[statusCol]}
                          />
                          <span className="text-xs font-mono text-slate-400">
                            {colTasks.length}
                          </span>
                        </div>

                        <div className="space-y-2.5 overflow-y-auto flex-1 pr-1">
                          {colTasks.map((taskItem) => (
                            <div
                              key={taskItem.id}
                              className="rounded-lg border border-slate-700/60 bg-slate-900/70 p-3 space-y-2 hover:border-blue-500/50 transition-colors"
                            >
                              <div className="flex items-center justify-between gap-1">
                                <span className="font-mono text-xs font-semibold text-blue-300">
                                  {taskItem.issue.identifier}
                                </span>
                                {taskItem.issue.priority ? (
                                  <SdkBadge
                                    variant="outline"
                                    color={
                                      taskItem.issue.priority <= 2
                                        ? "warning"
                                        : "secondary"
                                    }
                                    size="sm"
                                  >
                                    P{taskItem.issue.priority}
                                  </SdkBadge>
                                ) : null}
                              </div>

                              <div className="text-xs font-medium text-slate-100 line-clamp-2">
                                {taskItem.issue.title}
                              </div>

                              <div className="flex flex-wrap items-center gap-1.5">
                                <ProviderBadge
                                  provider={taskItem.assigned_provider}
                                  model={taskItem.assigned_model}
                                />
                                {taskItem.pr_number ? (
                                  <SdkBadge variant="soft" color="success" size="sm">
                                    PR #{taskItem.pr_number}
                                  </SdkBadge>
                                ) : null}
                              </div>

                              <div className="flex items-center justify-between text-[11px] text-slate-400 font-mono">
                                <span>{taskItem.total_tokens.toLocaleString()} tok</span>
                                <span>{taskItem.duration_seconds.toFixed(1)}s</span>
                              </div>

                              {taskItem.last_error ? (
                                <div className="text-[11px] text-red-400 bg-red-500/10 rounded px-2 py-1 truncate">
                                  {taskItem.last_error}
                                </div>
                              ) : null}

                              <div className="flex flex-wrap items-center justify-between gap-1 pt-1 border-t border-slate-800/80">
                                <select
                                  aria-label={`Move ${taskItem.issue.identifier} status`}
                                  className="rounded border border-slate-700 bg-slate-950 px-1.5 py-0.5 text-[11px] text-slate-200"
                                  value={taskItem.status}
                                  onChange={(e) =>
                                    statusTransitionMutation.mutate({
                                      id: taskItem.id,
                                      status: e.target.value as TaskStatus
                                    })
                                  }
                                >
                                  {ALL_TASK_STATUSES.filter((target) =>
                                    canTransitionTaskStatus(taskItem.status, target)
                                  ).map((target) => (
                                    <option key={target} value={target}>
                                      {t.status[target]}
                                    </option>
                                  ))}
                                </select>

                                <div className="flex items-center gap-1">
                                  {taskItem.status === "backlog" ||
                                  taskItem.status === "ready" ? (
                                    <SdkButton
                                      color="primary"
                                      variant="soft"
                                      size="2xs"
                                      onClick={() =>
                                        startTaskMutation.mutate(taskItem.id)
                                      }
                                    >
                                      Start
                                    </SdkButton>
                                  ) : null}
                                  {taskItem.status === "running" ? (
                                    <SdkButton
                                      color="danger"
                                      variant="ghost"
                                      size="2xs"
                                      onClick={() =>
                                        cancelTaskMutation.mutate(taskItem.id)
                                      }
                                    >
                                      Cancel
                                    </SdkButton>
                                  ) : null}
                                  {taskItem.status === "failed" ||
                                  taskItem.status === "cancelled" ? (
                                    <SdkButton
                                      color="warning"
                                      variant="soft"
                                      size="2xs"
                                      onClick={() =>
                                        retryTaskMutation.mutate(taskItem.id)
                                      }
                                    >
                                      Retry
                                    </SdkButton>
                                  ) : null}
                                  <SdkButton
                                    color="secondary"
                                    variant="ghost"
                                    size="2xs"
                                    onClick={() => {
                                      selectTask(
                                        taskItem.id,
                                        taskItem.issue.identifier
                                      );
                                      setActiveSection("executionView");
                                    }}
                                  >
                                    Inspect
                                  </SdkButton>
                                </div>
                              </div>
                            </div>
                          ))}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            ) : null}

            {/* SECTION D: AGENT CONTROL CENTER */}
            {activeSection === "agentControl" ? (
              <div className="space-y-5">
                {snapshot?.scheduler_paused ? (
                  <SdkAlert
                    color="warning"
                    variant="soft"
                    title={t.agents.schedulerPausedNotice}
                  />
                ) : null}

                {/* Active Agent Sessions */}
                <div className="space-y-3">
                  <h2 className="text-base font-semibold">
                    {t.agents.activeSessions} ({snapshot?.running.length ?? 0})
                  </h2>

                  {(snapshot?.running.length ?? 0) === 0 ? (
                    <AiEmptyView
                      title={t.agents.noActiveAgents}
                      description="Start any task from the Task Board or create a new task to launch a coding agent session."
                      actionLabel={t.tasks.newTask}
                      onAction={() => setNewTaskDialogOpen(true)}
                    />
                  ) : (
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                      {snapshot?.running.map((session) => (
                        <div
                          key={session.session_id}
                          className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-3"
                        >
                          <div className="flex items-center justify-between">
                            <div className="flex items-center gap-2">
                              <span className="font-mono text-sm font-bold text-blue-300">
                                {session.issue_identifier}
                              </span>
                              <ProviderBadge
                                provider={session.provider}
                                model={session.model}
                              />
                            </div>
                            <PhaseBadge phase={session.phase} />
                          </div>

                          <div className="text-xs text-slate-200">
                            Operation: {session.current_operation}
                          </div>

                          <div className="grid grid-cols-2 gap-2 text-xs text-slate-400 font-mono bg-slate-950/60 p-2.5 rounded-lg">
                            <div>Session: {session.session_id}</div>
                            <div>PID: {session.codex_app_server_pid ?? "N/A"}</div>
                            <div>Turn: #{session.turn_count}</div>
                            <div>
                              Tokens: {session.total_tokens.toLocaleString()}
                            </div>
                            <div className="col-span-2 truncate">
                              Worktree: {session.workspace_path}
                            </div>
                          </div>

                          {/* Explicit distinction between Scheduler Pause and Running Session Pause */}
                          {!session.supports_pause_session ? (
                            <div className="text-[11px] text-slate-400">
                              Note: {session.provider} JSON-RPC turns run atomically; use Scheduler Pause to halt new dispatches or Abort Session to interrupt this turn safely.
                            </div>
                          ) : null}

                          <div className="flex flex-wrap items-center justify-end gap-2 pt-1">
                            <SdkButton
                              color="secondary"
                              variant="outline"
                              size="xs"
                              onClick={() =>
                                void defaultSymphonyClient.openWorkspaceInExplorer(
                                  session.workspace_path
                                )
                              }
                            >
                              {t.agents.openWorkspace}
                            </SdkButton>
                            <SdkButton
                              color="primary"
                              variant="soft"
                              size="xs"
                              onClick={() => {
                                selectTask(
                                  session.task_id,
                                                                  session.issue_identifier
                                );
                                setActiveSection("executionView");
                              }}
                            >
                              {t.agents.openDetails}
                            </SdkButton>
                            <SdkButton
                              color="danger"
                              variant="solid"
                              size="xs"
                              onClick={() =>
                                cancelTaskMutation.mutate(session.task_id)
                              }
                            >
                              {t.agents.cancelExecution}
                            </SdkButton>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                {/* Registered Provider Runtimes & Capability Matrix */}
                <div className="space-y-3">
                  <h2 className="text-base font-semibold">
                    {t.agents.providerCapabilities}
                  </h2>
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    {providers.map((prof) => (
                      <div
                        key={prof.id}
                        className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-2.5"
                      >
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-2">
                            <ProviderBadge
                              provider={prof.provider}
                              model={prof.default_model}
                            />
                            <span className="text-xs font-semibold">
                              {prof.name}
                            </span>
                          </div>
                          <SdkBadge
                            variant="soft"
                            color={prof.installed ? "success" : "warning"}
                            size="sm"
                            pill
                          >
                            {prof.installed
                              ? `Installed (${prof.version ?? "ready"})`
                              : "CLI Not Detected"}
                          </SdkBadge>
                        </div>
                        <div className="text-xs font-mono text-slate-400">
                          Command: {prof.command}
                        </div>
                        <div className="flex flex-wrap gap-1.5 pt-1">
                          {prof.capabilities.supports_stream_events ? (
                            <SdkBadge variant="outline" color="info" size="sm">
                              Streaming JSON-RPC
                            </SdkBadge>
                          ) : null}
                          {prof.capabilities.supports_resume_session ? (
                            <SdkBadge variant="outline" color="info" size="sm">
                              Thread Resume
                            </SdkBadge>
                          ) : null}
                          {prof.capabilities.supports_approvals ? (
                            <SdkBadge variant="outline" color="success" size="sm">
                              Interactive Approvals
                            </SdkBadge>
                          ) : null}
                          {prof.capabilities.supports_dynamic_tools ? (
                            <SdkBadge variant="outline" color="discovery" size="sm">
                              Dynamic Host Tools
                            </SdkBadge>
                          ) : null}
                          {!prof.capabilities.supports_pause_session ? (
                            <SdkBadge variant="outline" color="secondary" size="sm">
                              Mid-Turn Pause Unsupported (Uses Turn Interrupt)
                            </SdkBadge>
                          ) : null}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            ) : null}

            {/* SECTION E: AGENT EXECUTION VIEW (3-Column Layout) */}
            {activeSection === "executionView" ? (
              <div className="space-y-4">
                {/* Task Selector Bar */}
                <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-slate-800 bg-[#131922] p-3">
                  <div className="flex items-center gap-3">
                    <span className="text-xs font-semibold text-slate-400">
                      Inspecting Task:
                    </span>
                    <select
                      aria-label="Select task to inspect"
                      className="rounded-md border border-slate-700 bg-slate-900 px-3 py-1.5 text-xs text-slate-100"
                      value={activeTask?.id ?? ""}
                      onChange={(e) => {
                        const found = tasks.find((t) => t.id === e.target.value);
                        if (found) {
                          selectTask(found.id, found.issue.identifier);
                        }
                      }}
                    >
                      {tasks.map((tItem) => (
                        <option key={tItem.id} value={tItem.id}>
                          {tItem.issue.identifier} — {tItem.issue.title} (
                          {tItem.status})
                        </option>
                      ))}
                    </select>
                    {activeTask ? (
                      <TaskStatusBadge
                        status={activeTask.status}
                        label={t.status[activeTask.status]}
                      />
                    ) : null}
                  </div>

                  <div className="flex items-center gap-2">
                    <LogModeSwitch
                      mode={logMode}
                      onChange={setLogMode}
                      conciseLabel={t.agents.conciseMode}
                      detailedLabel={t.agents.detailedLogsMode}
                    />
                    {activeTask &&
                    (activeTask.status === "backlog" ||
                      activeTask.status === "ready") ? (
                      <SdkButton
                        color="primary"
                        variant="solid"
                        size="xs"
                        onClick={() => startTaskMutation.mutate(activeTask.id)}
                      >
                        {t.tasks.startTask}
                      </SdkButton>
                    ) : null}
                    {activeTask && activeTask.status === "running" ? (
                      <SdkButton
                        color="danger"
                        variant="solid"
                        size="xs"
                        onClick={() => cancelTaskMutation.mutate(activeTask.id)}
                      >
                        {t.tasks.cancelTask}
                      </SdkButton>
                    ) : null}
                    {activeTask &&
                    (activeTask.status === "failed" ||
                      activeTask.status === "cancelled") ? (
                      <SdkButton
                        color="warning"
                        variant="soft"
                        size="xs"
                        onClick={() => retryTaskMutation.mutate(activeTask.id)}
                      >
                        {t.tasks.retryTask}
                      </SdkButton>
                    ) : null}
                  </div>
                </div>

                {!activeTask ? (
                  <AiEmptyView
                    title="No task selected"
                    description="Create or select a task from the Task Board to inspect real-time agent turns, diffs, and validation results."
                    actionLabel={t.tasks.newTask}
                    onAction={() => setNewTaskDialogOpen(true)}
                  />
                ) : (
                  <div className="grid grid-cols-1 lg:grid-cols-12 gap-4">
                    {/* LEFT COLUMN: Task Details & Context */}
                    <div className="lg:col-span-3 rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-4">
                      <div>
                        <div className="text-xs font-mono text-blue-400">
                          {activeTask.issue.identifier}
                        </div>
                        <h3 className="text-sm font-semibold mt-0.5">
                          {activeTask.issue.title}
                        </h3>
                      </div>

                      <div className="text-xs text-slate-300">
                        <SdkMarkdown breakNewLines>
                          {activeTask.issue.description ??
                            "No additional task description provided."}
                        </SdkMarkdown>
                      </div>

                      <div className="space-y-1.5 pt-2 border-t border-slate-800 text-xs font-mono text-slate-400">
                        <div>Branch: {activeTask.branch_name}</div>
                        <div>Workspace Key: {activeTask.workspace_key}</div>
                        <div className="break-all">
                          Worktree: {activeTask.workspace_path ?? "Not allocated yet"}
                        </div>
                        <div>Attempt: #{activeTask.current_attempt}</div>
                        <div>
                          Session ID: {activeTask.session_id ?? "None"}
                        </div>
                      </div>

                      {activeTask.workspace_path ? (
                        <SdkButton
                          color="secondary"
                          variant="outline"
                          size="xs"
                          block
                          onClick={() =>
                            void defaultSymphonyClient.openWorkspaceInExplorer(
                              activeTask.workspace_path!
                            )
                          }
                        >
                          {t.agents.openWorkspace}
                        </SdkButton>
                      ) : null}

                      {/* Attempt History */}
                      <div className="space-y-2 pt-2 border-t border-slate-800">
                        <div className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                          Run Attempts (
                          {issueDetailQuery.data?.attempts.history.length ?? 0})
                        </div>
                        {(issueDetailQuery.data?.attempts.history ?? []).map(
                          (att) => (
                            <div
                              key={att.id}
                              className="rounded border border-slate-800 bg-slate-950/60 p-2 text-xs space-y-1"
                            >
                              <div className="flex items-center justify-between">
                                <span className="font-mono">
                                  Attempt #{att.attempt}
                                </span>
                                <PhaseBadge phase={att.phase} />
                              </div>
                              <div className="text-[11px] text-slate-400 font-mono">
                                Turns: {att.turn_count} •{" "}
                                {att.total_tokens.toLocaleString()} tok
                              </div>
                            </div>
                          )
                        )}
                      </div>
                    </div>

                    {/* CENTER COLUMN: Agent Activity, Approvals & Conversation */}
                    <div className="lg:col-span-5 rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-3 flex flex-col max-h-[75vh]">
                      <div className="flex items-center justify-between">
                        <h3 className="text-sm font-semibold flex items-center gap-2">
                          <Bot className="w-4 h-4 text-blue-400" />
                          Agent Activity & Protocol Stream
                        </h3>
                        <ProviderBadge
                          provider={activeTask.assigned_provider}
                          model={activeTask.assigned_model}
                        />
                      </div>

                      {approvals
                        .filter((a) => a.task_id === activeTask.id)
                        .map((appItem) => (
                          <AiApprovalBanner
                            key={appItem.id}
                            approval={appItem}
                            onDecide={(id, decision) =>
                              resolveApprovalMutation.mutate({ id, decision })
                            }
                          />
                        ))}

                      <div className="flex-1 overflow-y-auto space-y-2.5 pr-1">
                        {(issueDetailQuery.data?.recent_events ?? []).length ===
                        0 ? (
                          <div className="text-xs text-slate-400 py-10 text-center">
                            No execution events recorded for this task yet.
                          </div>
                        ) : (
                          (issueDetailQuery.data?.recent_events ?? []).map(
                            (ev) => (
                              <AiEventStreamCard
                                key={ev.id}
                                event={ev}
                                detailedMode={logMode === "detailed"}
                              />
                            )
                          )
                        )}
                      </div>
                    </div>

                    {/* RIGHT COLUMN: Changed Files, Validation Results & Metadata */}
                    <div className="lg:col-span-4 rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-4 max-h-[75vh] overflow-y-auto">
                      <h3 className="text-sm font-semibold">
                        Validation, Changed Files & Telemetry
                      </h3>

                      {/* Token Telemetry */}
                      <div className="grid grid-cols-3 gap-2 text-center">
                        <div className="rounded-lg border border-slate-800 bg-slate-950/60 p-2">
                          <div className="text-[10px] text-slate-400">Input</div>
                          <div className="text-xs font-mono font-semibold">
                            {activeTask.total_input_tokens.toLocaleString()}
                          </div>
                        </div>
                        <div className="rounded-lg border border-slate-800 bg-slate-950/60 p-2">
                          <div className="text-[10px] text-slate-400">Output</div>
                          <div className="text-xs font-mono font-semibold">
                            {activeTask.total_output_tokens.toLocaleString()}
                          </div>
                        </div>
                        <div className="rounded-lg border border-slate-800 bg-slate-950/60 p-2">
                          <div className="text-[10px] text-slate-400">Est. Cost</div>
                          <div className="text-xs font-mono font-semibold text-emerald-400">
                            ${activeTask.estimated_cost_usd.toFixed(4)}
                          </div>
                        </div>
                      </div>

                      {/* Validation Results */}
                      <div className="space-y-2">
                        <div className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                          Validation Command Executions
                        </div>
                        {(issueDetailQuery.data?.validation_results ?? [])
                          .length === 0 ? (
                          <div className="text-xs text-slate-400">
                            Validation runs automatically after agent implementation turns complete.
                          </div>
                        ) : (
                          (
                            issueDetailQuery.data?.validation_results ?? []
                          ).map((val) => (
                            <div
                              key={val.id}
                              className="rounded-lg border border-slate-700/60 bg-slate-950/60 p-2.5 space-y-1.5"
                            >
                              <div className="flex items-center justify-between">
                                <span className="font-mono text-xs text-slate-200">
                                  {val.command}
                                </span>
                                <SdkBadge
                                  variant="soft"
                                  color={val.passed ? "success" : "danger"}
                                  size="sm"
                                >
                                  {val.passed
                                    ? `PASS (${val.duration_ms}ms)`
                                    : `EXIT ${val.exit_code}`}
                                </SdkBadge>
                              </div>
                              {val.stdout.trim() ? (
                                <SdkCodeBlock language="text">
                                  {val.stdout.trim()}
                                </SdkCodeBlock>
                              ) : null}
                              {val.stderr.trim() ? (
                                <SdkCodeBlock language="text">
                                  {val.stderr.trim()}
                                </SdkCodeBlock>
                              ) : null}
                            </div>
                          ))
                        )}
                      </div>

                      {/* Diff & PR Preview */}
                      {issueDetailQuery.data?.pull_request ? (
                        <div className="space-y-2 pt-2 border-t border-slate-800">
                          <div className="flex items-center justify-between">
                            <span className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                              Diff & Merge Readiness
                            </span>
                            <SdkBadge variant="solid" color="success" size="sm">
                              Score{" "}
                              {
                                issueDetailQuery.data.pull_request
                                  .merge_readiness.score
                              }
                              /5
                            </SdkBadge>
                          </div>
                          <div className="flex flex-wrap gap-1">
                            {issueDetailQuery.data.pull_request.changed_files.map(
                              (f) => (
                                <SdkBadge
                                  key={f}
                                  variant="outline"
                                  color="info"
                                  size="sm"
                                >
                                  {f}
                                </SdkBadge>
                              )
                            )}
                          </div>
                          {issueDetailQuery.data.pull_request.diff_preview ? (
                            <SdkCodeBlock language="diff">
                              {issueDetailQuery.data.pull_request.diff_preview}
                            </SdkCodeBlock>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  </div>
                )}
              </div>
            ) : null}

            {/* SECTION F: PULL REQUEST REVIEW */}
            {activeSection === "prReview" ? (
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <div>
                    <h2 className="text-base font-semibold">{t.prReview.title}</h2>
                    <p className="text-xs text-slate-400">
                      {t.prReview.humanApprovalRequired}
                    </p>
                  </div>
                </div>

                {prReviews.length === 0 ? (
                  <AiEmptyView
                    title="No Pull Requests Awaiting Review"
                    description="When an agent finishes implementing and validating a task, Symphony prepares a Pull Request review with a 1-to-5 evidence-backed Merge Readiness rating."
                    actionLabel={t.tasks.newTask}
                    onAction={() => setNewTaskDialogOpen(true)}
                  />
                ) : (
                  <div className="space-y-4">
                    {prReviews.map((review) => (
                      <MergeReadinessCard
                        key={review.id}
                        review={review}
                        onApproveMerge={(id) => approvePrMutation.mutate(id)}
                      />
                    ))}
                  </div>
                )}
              </div>
            ) : null}

            {/* SECTION G: WORKFLOW.MD STUDIO */}
            {activeSection === "workflowEditor" ? (
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <div>
                    <h2 className="text-base font-semibold">
                      WORKFLOW.md Repository Contract Studio
                    </h2>
                    <p className="text-xs text-slate-400">
                      Edit YAML front matter and Liquid prompt template with live Symphony SPEC.md validation and rendered prompt preview.
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    {workflowQuery.data?.valid ? (
                      <SdkBadge variant="soft" color="success" size="md" pill>
                        Valid WORKFLOW.md Contract
                      </SdkBadge>
                    ) : (
                      <SdkBadge variant="solid" color="danger" size="md" pill>
                        Validation Error
                      </SdkBadge>
                    )}
                    <SdkButton
                      color="primary"
                      variant="solid"
                      size="sm"
                      onClick={() =>
                        workflowDraft !== null &&
                        saveWorkflowMutation.mutate(workflowDraft)
                      }
                    >
                      Save & Hot-Reload WORKFLOW.md
                    </SdkButton>
                  </div>
                </div>

                {workflowQuery.data?.validation_error ? (
                  <SdkAlert
                    color="danger"
                    variant="soft"
                    title="WORKFLOW.md Validation Error"
                    description={workflowQuery.data.validation_error}
                  />
                ) : null}

                <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                  <div className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-2">
                    <div className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                      WORKFLOW.md Source ({workflowQuery.data?.path ?? "WORKFLOW.md"})
                    </div>
                    <textarea
                      aria-label="WORKFLOW.md editor"
                      className="w-full h-[460px] rounded-lg border border-slate-700 bg-slate-950 p-3 font-mono text-xs text-slate-100 leading-relaxed"
                      value={workflowDraft ?? ""}
                      onChange={(e) => setWorkflowDraft(e.target.value)}
                    />
                  </div>

                  <div className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-4">
                    <div className="space-y-2">
                      <div className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                        Rendered Task Prompt Preview (Strict Liquid)
                      </div>
                      <div className="rounded-lg border border-slate-800 bg-slate-950/80 p-3 text-xs">
                        <SdkMarkdown breakNewLines copyableCodeBlocks>
                          {workflowQuery.data?.rendered_prompt_preview ??
                            "Fix validation errors to preview rendered prompt."}
                        </SdkMarkdown>
                      </div>
                    </div>

                    {workflowQuery.data?.parsed_config ? (
                      <div className="space-y-2">
                        <div className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                          Effective Typed ServiceConfig
                        </div>
                        <SdkCodeBlock language="json">
                          {JSON.stringify(
                            workflowQuery.data.parsed_config,
                            null,
                            2
                          )}
                        </SdkCodeBlock>
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>
            ) : null}

            {/* SECTION H: SETTINGS */}
            {activeSection === "settings" && settingsQuery.data ? (
              <div className="space-y-5 max-w-4xl">
                <div className="flex items-center justify-between">
                  <div>
                    <h2 className="text-base font-semibold">{t.settings.title}</h2>
                    <p className="text-xs text-slate-400">
                      Configure Windows 11 background lifecycle, ASUS Zephyrus G16 power-aware concurrency, credentials vault, and remote API.
                    </p>
                  </div>
                  <SdkButton
                    color="secondary"
                    variant="outline"
                    size="sm"
                    onClick={() => setOnboardingOpen(true)}
                  >
                    Open Setup Wizard
                  </SdkButton>
                </div>

                {/* Appearance & Localization */}
                <div className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-4">
                  <h3 className="text-sm font-semibold flex items-center gap-2">
                    <Sliders className="w-4 h-4 text-blue-400" />
                    Appearance, Localization & Windows 11 Lifecycle
                  </h3>

                  <div className="grid grid-cols-1 md:grid-cols-3 gap-4 text-xs">
                    <div>
                      <label className="block text-slate-400 mb-1">
                        {t.settings.theme}
                      </label>
                      <select
                        aria-label={t.settings.theme}
                        className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                        value={settingsQuery.data.theme}
                        onChange={(e) =>
                          saveSettingsMutation.mutate({
                            ...settingsQuery.data!,
                            theme: e.target.value as "system" | "dark" | "light"
                          })
                        }
                      >
                        <option value="dark">Dark (Fluent Mica Dark)</option>
                        <option value="light">Light (Fluent Light)</option>
                        <option value="system">Synchronize with Windows 11</option>
                      </select>
                    </div>

                    <div>
                      <label className="block text-slate-400 mb-1">
                        {t.settings.language}
                      </label>
                      <select
                        aria-label={t.settings.language}
                        className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                        value={settingsQuery.data.language}
                        onChange={(e) =>
                          saveSettingsMutation.mutate({
                            ...settingsQuery.data!,
                            language: e.target.value as Locale
                          })
                        }
                      >
                        <option value="en">English (EN)</option>
                        <option value="sv">Svenska (SV)</option>
                      </select>
                    </div>

                    <div>
                      <label className="block text-slate-400 mb-1">
                        {t.settings.closeBehavior}
                      </label>
                      <select
                        aria-label={t.settings.closeBehavior}
                        className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                        value={settingsQuery.data.close_action}
                        onChange={(e) =>
                          saveSettingsMutation.mutate({
                            ...settingsQuery.data!,
                            close_action: e.target.value as
                              | "minimize_to_tray"
                              | "exit_application"
                          })
                        }
                      >
                        <option value="minimize_to_tray">
                          {t.settings.minimizeToTray}
                        </option>
                        <option value="exit_application">
                          {t.settings.exitApplication}
                        </option>
                      </select>
                    </div>
                  </div>

                  <div className="flex flex-wrap items-center gap-3 pt-2 border-t border-slate-800">
                    <SdkButton
                      color="warning"
                      variant="soft"
                      size="xs"
                      onClick={() =>
                        void defaultSymphonyClient.stopBackgroundOrchestrator()
                      }
                    >
                      {t.settings.stopOrchestrator}
                    </SdkButton>
                    <SdkButton
                      color="danger"
                      variant="outline"
                      size="xs"
                      onClick={() =>
                        void defaultSymphonyClient.exitApplicationCompletely()
                      }
                    >
                      {t.settings.exitApplication}
                    </SdkButton>
                  </div>
                </div>

                {/* Hardware-Aware Resource Policy */}
                <div className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-4">
                  <h3 className="text-sm font-semibold">
                    Hardware-Aware Scheduling (ASUS Zephyrus G16 / Ryzen AI 9 HX 370 Profile)
                  </h3>
                  <div className="grid grid-cols-1 md:grid-cols-3 gap-4 text-xs">
                    <div>
                      <label className="block text-slate-400 mb-1">
                        {t.settings.acConcurrency}
                      </label>
                      <Input
                        type="number"
                        value={String(
                          settingsQuery.data.resource_policy.max_agents_ac
                        )}
                        onChange={(_, data) =>
                          saveSettingsMutation.mutate({
                            ...settingsQuery.data!,
                            resource_policy: {
                              ...settingsQuery.data!.resource_policy,
                              max_agents_ac: Math.max(
                                1,
                                Number(data.value) || 2
                              )
                            }
                          })
                        }
                      />
                    </div>
                    <div>
                      <label className="block text-slate-400 mb-1">
                        {t.settings.batteryConcurrency}
                      </label>
                      <Input
                        type="number"
                        value={String(
                          settingsQuery.data.resource_policy.max_agents_battery
                        )}
                        onChange={(_, data) =>
                          saveSettingsMutation.mutate({
                            ...settingsQuery.data!,
                            resource_policy: {
                              ...settingsQuery.data!.resource_policy,
                              max_agents_battery: Math.max(
                                1,
                                Number(data.value) || 1
                              )
                            }
                          })
                        }
                      />
                    </div>
                    <div>
                      <label className="block text-slate-400 mb-1">
                        Memory Pressure Threshold (%)
                      </label>
                      <Input
                        type="number"
                        value={String(
                          settingsQuery.data.resource_policy
                            .memory_pressure_threshold_percent
                        )}
                        onChange={(_, data) =>
                          saveSettingsMutation.mutate({
                            ...settingsQuery.data!,
                            resource_policy: {
                              ...settingsQuery.data!.resource_policy,
                              memory_pressure_threshold_percent: Math.min(
                                98,
                                Math.max(50, Number(data.value) || 85)
                              )
                            }
                          })
                        }
                      />
                    </div>
                  </div>
                </div>

                {/* Isolated Secret Vault */}
                <div className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-3">
                  <h3 className="text-sm font-semibold flex items-center gap-2">
                    <Shield className="w-4 h-4 text-emerald-400" />
                    Host-Isolated Credential Vault (Never Exposed to Agent Subprocesses)
                  </h3>
                  <p className="text-xs text-slate-400">
                    Tracker tokens are stored outside SQLite and explicitly stripped from coding-agent subprocess environments.
                  </p>
                  <div className="flex flex-wrap items-center gap-2">
                    <select
                      aria-label="Credential key"
                      className="rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-xs"
                      value={credKey}
                      onChange={(e) => setCredKey(e.target.value)}
                    >
                      <option value="GITHUB_TOKEN">GITHUB_TOKEN</option>
                      <option value="LINEAR_API_KEY">LINEAR_API_KEY</option>
                      <option value="SYMPHONY_REMOTE_TOKEN">
                        SYMPHONY_REMOTE_TOKEN (iOS Companion)
                      </option>
                    </select>
                    <input
                      type="password"
                      placeholder="Paste token value..."
                      className="flex-1 rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-xs"
                      value={credValue}
                      onChange={(e) => setCredValue(e.target.value)}
                    />
                    <SdkButton
                      color="primary"
                      variant="solid"
                      size="sm"
                      disabled={!credValue.trim()}
                      onClick={() => saveCredentialMutation.mutate()}
                    >
                      Store in Vault
                    </SdkButton>
                  </div>
                </div>

                {/* Diagnostic Export */}
                <div className="rounded-xl border border-slate-800 bg-[#131922] p-4 space-y-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <h3 className="text-sm font-semibold">
                        Redacted Diagnostic Bundle
                      </h3>
                      <p className="text-xs text-slate-400">
                        Export orchestrator state, tasks, and events with automatic secret redaction.
                      </p>
                    </div>
                    <SdkButton
                      color="secondary"
                      variant="outline"
                      size="sm"
                      onClick={async () => {
                        const bundle =
                          await defaultSymphonyClient.exportDiagnostics();
                        setDiagnosticJson(JSON.stringify(bundle, null, 2));
                      }}
                    >
                      {t.settings.exportDiagnostics}
                    </SdkButton>
                  </div>
                  {diagnosticJson ? (
                    <SdkCodeBlock language="json">{diagnosticJson}</SdkCodeBlock>
                  ) : null}
                </div>
              </div>
            ) : null}
          </main>
        </div>

        {/* NEW TASK DIALOG */}
        <Dialog
          open={newTaskDialogOpen}
          onOpenChange={(_, data) => setNewTaskDialogOpen(data.open)}
        >
          <DialogSurface className="bg-[#131922] text-slate-100 border border-slate-700 max-w-lg w-full">
            <DialogBody>
              <DialogTitle>{t.tasks.newTask}</DialogTitle>
              <DialogContent className="space-y-3 pt-2">
                <div className="grid grid-cols-2 gap-2 text-xs">
                  <div>
                    <label className="block text-slate-400 mb-1">
                      Identifier (Optional)
                    </label>
                    <input
                      type="text"
                      placeholder="e.g. SYM-101"
                      className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-xs"
                      value={taskIdentifier}
                      onChange={(e) => setTaskIdentifier(e.target.value)}
                    />
                  </div>
                  <div>
                    <label className="block text-slate-400 mb-1">
                      {t.tasks.priority}
                    </label>
                    <select
                      aria-label={t.tasks.priority}
                      className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-xs"
                      value={taskPriority}
                      onChange={(e) => setTaskPriority(Number(e.target.value))}
                    >
                      <option value={1}>1 — Urgent</option>
                      <option value={2}>2 — High</option>
                      <option value={3}>3 — Normal</option>
                      <option value={4}>4 — Low</option>
                    </select>
                  </div>
                </div>

                <div className="text-xs">
                  <label className="block text-slate-400 mb-1">
                    {t.tasks.title}
                  </label>
                  <input
                    type="text"
                    placeholder="Implement feature or fix bug..."
                    className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-xs"
                    value={taskTitle}
                    onChange={(e) => setTaskTitle(e.target.value)}
                  />
                </div>

                <div className="text-xs">
                  <label className="block text-slate-400 mb-1">
                    {t.tasks.provider}
                  </label>
                  <select
                    aria-label={t.tasks.provider}
                    className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-xs"
                    value={taskProvider}
                    onChange={(e) =>
                      setTaskProvider(e.target.value as ProviderKind)
                    }
                  >
                    <option value="codex">OpenAI Codex App Server</option>
                    <option value="claude_code">Claude Code</option>
                    <option value="opencode">OpenCode</option>
                    <option value="simulated">
                      Simulated Provider (Deterministic Verification)
                    </option>
                  </select>
                </div>

                <div className="text-xs">
                  <label className="block text-slate-400 mb-1">
                    {t.tasks.description}
                  </label>
                  <PromptComposerBox
                    value={taskDescription}
                    onChange={setTaskDescription}
                  />
                </div>

                <div className="flex items-center gap-2 pt-1">
                  <Switch
                    checked={taskDispatchNow}
                    onChange={(_, data) => setTaskDispatchNow(data.checked)}
                    label={t.tasks.dispatchImmediately}
                  />
                </div>
              </DialogContent>
              <DialogActions>
                <FluentButton
                  appearance="secondary"
                  onClick={() => setNewTaskDialogOpen(false)}
                >
                  Cancel
                </FluentButton>
                <SdkButton
                  color="primary"
                  variant="solid"
                  size="sm"
                  disabled={!taskTitle.trim()}
                  onClick={() => createTaskMutation.mutate()}
                >
                  {t.tasks.createTask}
                </SdkButton>
              </DialogActions>
            </DialogBody>
          </DialogSurface>
        </Dialog>

        {/* NEW PROJECT DIALOG */}
        <Dialog
          open={newProjectDialogOpen}
          onOpenChange={(_, data) => setNewProjectDialogOpen(data.open)}
        >
          <DialogSurface className="bg-[#131922] text-slate-100 border border-slate-700 max-w-lg w-full">
            <DialogBody>
              <DialogTitle>Register Project & Repository</DialogTitle>
              <DialogContent className="space-y-3 pt-2 text-xs">
                <div>
                  <label className="block text-slate-400 mb-1">Project Name</label>
                  <input
                    type="text"
                    placeholder="My Repository"
                    className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                    value={projName}
                    onChange={(e) => setProjName(e.target.value)}
                  />
                </div>
                <div className="grid grid-cols-3 gap-2">
                  <div className="col-span-2">
                    <label className="block text-slate-400 mb-1">
                      Local Git Repository Path
                    </label>
                    <input
                      type="text"
                      placeholder="C:\Users\kevin\Projekt\MyRepo"
                      className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                      value={projRepoPath}
                      onChange={(e) => setProjRepoPath(e.target.value)}
                    />
                  </div>
                  <div>
                    <label className="block text-slate-400 mb-1">
                      Default Branch
                    </label>
                    <input
                      type="text"
                      className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                      value={projDefaultBranch}
                      onChange={(e) => setProjDefaultBranch(e.target.value)}
                    />
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className="block text-slate-400 mb-1">
                      Tracker Integration
                    </label>
                    <select
                      aria-label="Tracker Integration"
                      className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                      value={projTrackerKind}
                      onChange={(e) => setProjTrackerKind(e.target.value)}
                    >
                      <option value="github">GitHub Issues</option>
                      <option value="linear">Linear</option>
                      <option value="manual">Manual Tasks Only</option>
                    </select>
                  </div>
                  <div>
                    <label className="block text-slate-400 mb-1">
                      Preferred Agent Provider
                    </label>
                    <select
                      aria-label="Preferred Agent Provider"
                      className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                      value={projProvider}
                      onChange={(e) =>
                        setProjProvider(e.target.value as ProviderKind)
                      }
                    >
                      <option value="codex">OpenAI Codex</option>
                      <option value="claude_code">Claude Code</option>
                      <option value="opencode">OpenCode</option>
                      <option value="simulated">Simulated</option>
                    </select>
                  </div>
                </div>
                {projTrackerKind === "github" ? (
                  <div>
                    <label className="block text-slate-400 mb-1">
                      GitHub Repository (owner/repo)
                    </label>
                    <input
                      type="text"
                      placeholder="openai/symphony"
                      className="w-full rounded border border-slate-700 bg-slate-900 px-2.5 py-1.5"
                      value={projGithubRepo}
                      onChange={(e) => setProjGithubRepo(e.target.value)}
                    />
                  </div>
                ) : null}
                <div>
                  <label className="block text-slate-400 mb-1">
                    Validation Commands (One per line)
                  </label>
                  <textarea
                    rows={2}
                    className="w-full rounded border border-slate-700 bg-slate-900 p-2 font-mono"
                    value={projValidationCmds}
                    onChange={(e) => setProjValidationCmds(e.target.value)}
                  />
                </div>
                <Switch
                  checked={projAutoDispatch}
                  onChange={(_, data) => setProjAutoDispatch(data.checked)}
                  label="Opt-in to automatic issue claiming for matching required labels"
                />
              </DialogContent>
              <DialogActions>
                <FluentButton
                  appearance="secondary"
                  onClick={() => setNewProjectDialogOpen(false)}
                >
                  Cancel
                </FluentButton>
                <SdkButton
                  color="primary"
                  variant="solid"
                  size="sm"
                  disabled={!projName.trim() || !projRepoPath.trim()}
                  onClick={() => createProjectMutation.mutate()}
                >
                  Register Project
                </SdkButton>
              </DialogActions>
            </DialogBody>
          </DialogSurface>
        </Dialog>

        {/* COMMAND PALETTE (Ctrl+K) */}
        <Dialog
          open={commandPaletteOpen}
          onOpenChange={(_, data) => setCommandPaletteOpen(data.open)}
        >
          <DialogSurface className="bg-[#131922] text-slate-100 border border-slate-700 max-w-md w-full">
            <DialogBody>
              <DialogTitle className="flex items-center gap-2 text-sm">
                <Search className="w-4 h-4 text-blue-400" />
                Command Palette
              </DialogTitle>
              <DialogContent className="space-y-2 pt-2">
                <input
                  type="text"
                  autoFocus
                  placeholder="Jump to view or trigger orchestrator action..."
                  className="w-full rounded border border-slate-700 bg-slate-900 px-3 py-2 text-xs"
                  value={commandSearch}
                  onChange={(e) => setCommandSearch(e.target.value)}
                />
                <div className="space-y-1 max-h-60 overflow-y-auto">
                  {navItems
                    .filter((item) =>
                      item.label
                        .toLowerCase()
                        .includes(commandSearch.toLowerCase())
                    )
                    .map((item) => (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => {
                          setActiveSection(item.id);
                          setCommandPaletteOpen(false);
                        }}
                        className="w-full flex items-center justify-between px-3 py-2 rounded hover:bg-slate-800 text-xs"
                      >
                        <span className="flex items-center gap-2">
                          {item.icon}
                          {item.label}
                        </span>
                        <span className="text-[10px] text-slate-400">Jump</span>
                      </button>
                    ))}
                </div>
              </DialogContent>
            </DialogBody>
          </DialogSurface>
        </Dialog>

        {/* FIRST-RUN ONBOARDING WIZARD */}
        <Dialog
          open={onboardingOpen}
          onOpenChange={(_, data) => setOnboardingOpen(data.open)}
        >
          <DialogSurface className="bg-[#131922] text-slate-100 border border-slate-700 max-w-xl w-full">
            <DialogBody>
              <DialogTitle className="flex items-center gap-2">
                <Sparkles className="w-5 h-5 text-blue-400" />
                {t.settings.onboardingTitle}
              </DialogTitle>
              <DialogContent className="space-y-3 pt-2 text-xs">
                <p className="text-slate-300">
                  Welcome to <strong>Symphony Desktop 1.0</strong> — your native Windows 11 AI coding orchestration platform built on Rust, Tauri 2, Fluent UI 2, and OpenAI Apps SDK UI.
                </p>
                <div className="space-y-2 rounded-lg border border-slate-800 bg-slate-950/60 p-3">
                  <div className="font-semibold text-slate-200">
                    Detected Coding Agent Providers:
                  </div>
                  {providers.map((p) => (
                    <div
                      key={p.id}
                      className="flex items-center justify-between"
                    >
                      <span>{p.name}</span>
                      <SdkBadge
                        variant="soft"
                        color={p.installed ? "success" : "warning"}
                        size="sm"
                      >
                        {p.installed ? p.version ?? "Installed" : "Not Found"}
                      </SdkBadge>
                    </div>
                  ))}
                </div>
                <ProgressBar value={1} />
              </DialogContent>
              <DialogActions>
                <SdkButton
                  color="primary"
                  variant="solid"
                  size="sm"
                  onClick={() => {
                    setOnboardingOpen(false);
                    if (settingsQuery.data) {
                      saveSettingsMutation.mutate({
                        ...settingsQuery.data,
                        onboarding_completed: true
                      });
                    }
                  }}
                >
                  Complete Setup & Launch Studio
                </SdkButton>
              </DialogActions>
            </DialogBody>
          </DialogSurface>
        </Dialog>
      </div>
    </SymphonyDesignProvider>
  );
};
