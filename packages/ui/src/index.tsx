import React, { useEffect, useMemo, useState } from "react";
import {
  FluentProvider,
  webDarkTheme,
  webLightTheme,
  type Theme
} from "@fluentui/react-components";
import { AppsSDKUIProvider } from "@openai/apps-sdk-ui/components/AppsSDKUIProvider";
import { Alert } from "@openai/apps-sdk-ui/components/Alert";
import { Badge } from "@openai/apps-sdk-ui/components/Badge";
import { Button as SdkButton, CopyButton } from "@openai/apps-sdk-ui/components/Button";
import { CodeBlock } from "@openai/apps-sdk-ui/components/CodeBlock";
import { EmptyMessage } from "@openai/apps-sdk-ui/components/EmptyMessage";
import { Markdown } from "@openai/apps-sdk-ui/components/Markdown";
import { Popover } from "@openai/apps-sdk-ui/components/Popover";
import { SegmentedControl } from "@openai/apps-sdk-ui/components/SegmentedControl";
import { ShimmerText } from "@openai/apps-sdk-ui/components/ShimmerText";
import { Textarea } from "@openai/apps-sdk-ui/components/Textarea";
import { Tooltip } from "@openai/apps-sdk-ui/components/Tooltip";
import { applyDocumentTheme } from "@openai/apps-sdk-ui/theme";
import type {
  AgentEvent,
  ApprovalDecision,
  ApprovalRequest,
  HardwareSnapshot,
  MergeReadinessReport,
  ProviderKind,
  PullRequestReview,
  RunAttemptPhase,
  TaskStatus
} from "@symphony/shared-types";
import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  Cpu,
  FileCode2,
  GitPullRequest,
  Info,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Terminal,
  XCircle
} from "lucide-react";

export {
  Alert as SdkAlert,
  Badge as SdkBadge,
  SdkButton,
  CopyButton as SdkCopyButton,
  CodeBlock as SdkCodeBlock,
  EmptyMessage as SdkEmptyMessage,
  Markdown as SdkMarkdown,
  Popover as SdkPopover,
  SegmentedControl as SdkSegmentedControl,
  ShimmerText as SdkShimmerText,
  Textarea as SdkTextarea,
  Tooltip as SdkTooltip
};

const symphonyDarkTheme: Theme = {
  ...webDarkTheme,
  fontFamilyBase:
    "'Segoe UI Variable Text', 'Segoe UI', -apple-system, BlinkMacSystemFont, sans-serif",
  fontFamilyMonospace: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace",
  colorNeutralBackground1: "#0d1117",
  colorNeutralBackground2: "#131922",
  colorNeutralBackground3: "#19212e",
  colorBrandBackground: "#2563eb",
  colorBrandBackgroundHover: "#1d4ed8",
  borderRadiusMedium: "8px",
  borderRadiusLarge: "12px"
};

const symphonyLightTheme: Theme = {
  ...webLightTheme,
  fontFamilyBase:
    "'Segoe UI Variable Text', 'Segoe UI', -apple-system, BlinkMacSystemFont, sans-serif",
  fontFamilyMonospace: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace",
  colorNeutralBackground1: "#f8fafc",
  colorNeutralBackground2: "#ffffff",
  colorNeutralBackground3: "#f1f5f9",
  colorBrandBackground: "#2563eb",
  colorBrandBackgroundHover: "#1d4ed8",
  borderRadiusMedium: "8px",
  borderRadiusLarge: "12px"
};

export interface SymphonyDesignProviderProps {
  themeMode: "system" | "dark" | "light";
  children: React.ReactNode;
}

export const SymphonyDesignProvider: React.FC<SymphonyDesignProviderProps> = ({
  themeMode,
  children
}) => {
  const [systemDark, setSystemDark] = useState<boolean>(() => {
    if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
      return window.matchMedia("(prefers-color-scheme: dark)").matches;
    }
    return true;
  });

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return;
    }
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = (ev: MediaQueryListEvent) => setSystemDark(ev.matches);
    mq.addEventListener?.("change", handler);
    return () => mq.removeEventListener?.("change", handler);
  }, []);

  const resolvedTheme: "dark" | "light" =
    themeMode === "system" ? (systemDark ? "dark" : "light") : themeMode;

  useEffect(() => {
    try {
      applyDocumentTheme(resolvedTheme);
    } catch {
      if (typeof document !== "undefined") {
        document.documentElement.setAttribute("data-theme", resolvedTheme);
        document.documentElement.style.colorScheme = resolvedTheme;
      }
    }
  }, [resolvedTheme]);

  const fluentTheme = useMemo(
    () => (resolvedTheme === "dark" ? symphonyDarkTheme : symphonyLightTheme),
    [resolvedTheme]
  );

  return (
    <FluentProvider theme={fluentTheme} className="symphony-fluent-root">
      <AppsSDKUIProvider linkComponent="a">{children}</AppsSDKUIProvider>
    </FluentProvider>
  );
};

export const ProviderBadge: React.FC<{
  provider: ProviderKind;
  model?: string | null;
}> = ({ provider, model }) => {
  const colorMap: Record<
    ProviderKind,
    "discovery" | "info" | "success" | "secondary"
  > = {
    codex: "discovery",
    claude_code: "info",
    opencode: "success",
    simulated: "secondary"
  };

  const labelMap: Record<ProviderKind, string> = {
    codex: "OpenAI Codex",
    claude_code: "Claude Code",
    opencode: "OpenCode",
    simulated: "Simulated"
  };

  return (
    <span className="inline-flex items-center gap-1.5">
      <Badge variant="soft" color={colorMap[provider] ?? "secondary"} size="sm" pill>
        {labelMap[provider] ?? provider}
      </Badge>
      {model ? (
        <Badge variant="outline" color="secondary" size="sm">
          {model}
        </Badge>
      ) : null}
    </span>
  );
};

export const TaskStatusBadge: React.FC<{
  status: TaskStatus;
  label?: string;
}> = ({ status, label }) => {
  const colorMap: Record<
    TaskStatus,
    "secondary" | "info" | "discovery" | "warning" | "success" | "danger"
  > = {
    backlog: "secondary",
    ready: "info",
    queued: "discovery",
    running: "info",
    blocked: "warning",
    human_review: "discovery",
    completed: "success",
    failed: "danger",
    cancelled: "secondary"
  };

  return (
    <span className="inline-flex items-center gap-1.5">
      <Badge variant="soft" color={colorMap[status] ?? "secondary"} size="sm" pill>
        {status === "running" ? (
          <ShimmerText as="span" className="font-medium">
            {label ?? status}
          </ShimmerText>
        ) : (
          label ?? status
        )}
      </Badge>
    </span>
  );
};

export const PhaseBadge: React.FC<{ phase: RunAttemptPhase }> = ({ phase }) => {
  const isStreaming =
    phase === "streaming_turn" ||
    phase === "initializing_session" ||
    phase === "launching_agent_process" ||
    phase === "preparing_workspace";

  const color =
    phase === "succeeded"
      ? "success"
      : phase === "failed" || phase === "timed_out" || phase === "stalled"
        ? "danger"
        : isStreaming
          ? "info"
          : "secondary";

  return (
    <Badge variant="outline" color={color} size="sm" pill>
      {isStreaming ? (
        <ShimmerText as="span">{phase.replace(/_/g, " ")}</ShimmerText>
      ) : (
        phase.replace(/_/g, " ")
      )}
    </Badge>
  );
};

export const AiApprovalBanner: React.FC<{
  approval?: ApprovalRequest;
  request?: ApprovalRequest;
  isSubmitting?: boolean;
  onDecide: (id: string, decision: ApprovalDecision) => void;
}> = ({ approval, request, isSubmitting = false, onDecide }) => {
  const item = approval ?? request;
  if (!item) return null;

  return (
    <div className="rounded-xl border border-amber-500/40 bg-amber-500/5 p-3.5 space-y-3">
      <Alert
        color="warning"
        variant="soft"
        title={`Approval Required • ${item.issue_identifier} (${item.kind})`}
        description={
          item.reason ??
          "The coding agent requested host authorization before executing this operation."
        }
      />
      {item.command ? (
        <CodeBlock language="bash">{item.command}</CodeBlock>
      ) : null}
      {item.file_paths.length > 0 ? (
        <div className="flex flex-wrap gap-1.5">
          {item.file_paths.map((fp) => (
            <Badge key={fp} variant="outline" color="warning" size="sm">
              {fp}
            </Badge>
          ))}
        </div>
      ) : null}
      <div className="flex flex-wrap items-center gap-2 pt-1">
        <SdkButton
          color="primary"
          variant="solid"
          size="sm"
          disabled={isSubmitting}
          onClick={() => onDecide(item.id, "approve")}
        >
          Approve Once
        </SdkButton>
        <SdkButton
          color="discovery"
          variant="soft"
          size="sm"
          disabled={isSubmitting}
          onClick={() => onDecide(item.id, "approve_for_session")}
        >
          Approve for Session
        </SdkButton>
        <SdkButton
          color="secondary"
          variant="outline"
          size="sm"
          disabled={isSubmitting}
          onClick={() => onDecide(item.id, "deny")}
        >
          Decline
        </SdkButton>
        <SdkButton
          color="danger"
          variant="ghost"
          size="sm"
          disabled={isSubmitting}
          onClick={() => onDecide(item.id, "abort")}
        >
          Abort Turn
        </SdkButton>
      </div>
    </div>
  );
};

export const AiEventStreamCard: React.FC<{
  event: AgentEvent;
  detailedMode?: boolean;
  detailed?: boolean;
}> = ({ event, detailedMode = false, detailed = false }) => {
  const showDetailed = detailedMode || detailed;
  const textContent = event.summary ?? event.message ?? "";
  const rawPayload = event.payload ?? event.raw_payload;
  const isError =
    event.event_type.includes("failed") ||
    event.event_type.includes("error") ||
    event.phase === "failed";
  const isTool =
    event.event_type.includes("tool") || event.event_type.includes("command");

  return (
    <div
      className={`rounded-lg border p-3 transition-colors ${
        isError
          ? "border-red-500/30 bg-red-500/5"
          : "border-slate-700/50 bg-slate-900/40"
      }`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2 mb-1.5">
        <div className="flex items-center gap-2">
          {isError ? (
            <XCircle className="w-4 h-4 text-red-400 shrink-0" />
          ) : isTool ? (
            <Terminal className="w-4 h-4 text-sky-400 shrink-0" />
          ) : (
            <Bot className="w-4 h-4 text-indigo-400 shrink-0" />
          )}
          <Badge variant="soft" color={isError ? "danger" : "info"} size="sm">
            {event.issue_identifier}
          </Badge>
          <span className="text-xs font-mono text-slate-400">{event.event_type}</span>
          {event.phase ? <PhaseBadge phase={event.phase} /> : null}
        </div>
        <div className="flex items-center gap-2 text-xs text-slate-400">
          {event.usage ? (
            <Badge variant="outline" color="secondary" size="sm">
              {event.usage.total_tokens.toLocaleString()} tok
            </Badge>
          ) : null}
          <span>{new Date(event.timestamp).toLocaleTimeString()}</span>
          <CopyButton copyValue={textContent} color="secondary" size="2xs" variant="ghost" />
        </div>
      </div>

      <div className="text-sm leading-relaxed text-slate-200">
        <Markdown breakNewLines copyableCodeBlocks>
          {textContent}
        </Markdown>
      </div>

      {showDetailed && rawPayload ? (
        <div className="mt-2">
          <Popover>
            <Popover.Trigger>
              <SdkButton variant="ghost" color="secondary" size="2xs">
                Inspect JSON-RPC Payload
              </SdkButton>
            </Popover.Trigger>
            <Popover.Content width={520} side="bottom" align="start">
              <div className="p-2">
                <CodeBlock language="json">
                  {JSON.stringify(rawPayload, null, 2)}
                </CodeBlock>
              </div>
            </Popover.Content>
          </Popover>
        </div>
      ) : null}
    </div>
  );
};

export const MergeReadinessCard: React.FC<{
  review: PullRequestReview;
  isApproving?: boolean;
  onApproveMerge?: (reviewId: string) => void;
}> = ({ review, isApproving = false, onApproveMerge }) => {
  const report: MergeReadinessReport = review.merge_readiness;
  const badgeColor =
    report.score >= 4 ? "success" : report.score === 3 ? "warning" : "danger";

  return (
    <div className="rounded-xl border border-slate-700/60 bg-slate-900/50 p-4 space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <GitPullRequest className="w-5 h-5 text-blue-400" />
            <h3 className="text-base font-semibold">
              PR #{review.pr_number}: {review.title}
            </h3>
            <Badge variant="solid" color={badgeColor} size="md" pill>
              Readiness {report.score} / 5
            </Badge>
            {review.human_approved ? (
              <Badge variant="soft" color="success" size="sm" pill>
                Human Approved
              </Badge>
            ) : (
              <Badge variant="outline" color="warning" size="sm" pill>
                Awaiting Human Approval
              </Badge>
            )}
          </div>
          <p className="text-xs text-slate-400 mt-1 font-mono">
            {review.head_branch} → {review.base_branch} • CI: {review.ci_status}
          </p>
        </div>

        {onApproveMerge && !review.human_approved ? (
          <Tooltip content="Explicit human authorization required before completing or merging">
            <SdkButton
              color="success"
              variant="solid"
              size="sm"
              disabled={isApproving}
              onClick={() => onApproveMerge(review.id)}
            >
              Approve & Complete PR
            </SdkButton>
          </Tooltip>
        ) : null}
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-xs">
        <div className="rounded-lg border border-slate-700/50 p-2.5 flex items-center gap-2">
          {report.ci_passing ? (
            <CheckCircle2 className="w-4 h-4 text-emerald-400" />
          ) : (
            <AlertTriangle className="w-4 h-4 text-amber-400" />
          )}
          <span>CI Status: {report.ci_passing ? "Passing" : "Pending/Failed"}</span>
        </div>
        <div className="rounded-lg border border-slate-700/50 p-2.5 flex items-center gap-2">
          {report.validation_passing ? (
            <CheckCircle2 className="w-4 h-4 text-emerald-400" />
          ) : (
            <XCircle className="w-4 h-4 text-red-400" />
          )}
          <span>
            Validation: {report.validation_passing ? "Verified" : "Failed"}
          </span>
        </div>
        <div className="rounded-lg border border-slate-700/50 p-2.5 flex items-center gap-2">
          {report.no_merge_conflicts ? (
            <CheckCircle2 className="w-4 h-4 text-emerald-400" />
          ) : (
            <XCircle className="w-4 h-4 text-red-400" />
          )}
          <span>
            Conflicts: {report.no_merge_conflicts ? "Clean" : "Conflict Detected"}
          </span>
        </div>
        <div className="rounded-lg border border-slate-700/50 p-2.5 flex items-center gap-2">
          {report.security_clean ? (
            <ShieldCheck className="w-4 h-4 text-emerald-400" />
          ) : (
            <ShieldAlert className="w-4 h-4 text-red-400" />
          )}
          <span>
            Security Scan: {report.security_clean ? "Clean" : "Warnings"}
          </span>
        </div>
      </div>

      {report.rationale.length > 0 ? (
        <div className="space-y-1">
          <div className="text-xs font-semibold uppercase tracking-wider text-slate-400">
            Evidence & Rationale
          </div>
          <ul className="text-xs space-y-1 text-slate-300 list-disc list-inside">
            {report.rationale.map((item, idx) => (
              <li key={idx}>{item}</li>
            ))}
          </ul>
        </div>
      ) : null}

      {report.warnings.length > 0 ? (
        <Alert
          color="warning"
          variant="soft"
          title="Readiness Warnings"
          description={report.warnings.join(" • ")}
        />
      ) : null}

      {review.comment_evaluations.length > 0 ? (
        <div className="space-y-2">
          <div className="text-xs font-semibold uppercase tracking-wider text-slate-400">
            Evaluated Review Comments (Verified Against Code Diff)
          </div>
          {review.comment_evaluations.map((comment, idx) => (
            <div
              key={idx}
              className="rounded-lg border border-slate-700/40 bg-slate-950/50 p-2.5 text-xs space-y-1"
            >
              <div className="flex items-center justify-between">
                <span className="font-medium text-slate-200">
                  {comment.author}{" "}
                  {comment.is_agent ? (
                    <Badge variant="soft" color="discovery" size="sm">
                      Review Agent
                    </Badge>
                  ) : null}
                </span>
                <Badge
                  variant="outline"
                  color={comment.verified_against_diff ? "success" : "warning"}
                  size="sm"
                >
                  {comment.verified_against_diff
                    ? "Verified in Diff"
                    : "Outdated / Unverified"}
                </Badge>
              </div>
              <p className="text-slate-300">{comment.body}</p>
              <p className="text-slate-400 italic">{comment.evaluation_summary}</p>
            </div>
          ))}
        </div>
      ) : null}

      {review.changed_files.length > 0 ? (
        <div className="space-y-1.5">
          <div className="text-xs font-semibold uppercase tracking-wider text-slate-400 flex items-center gap-1.5">
            <FileCode2 className="w-3.5 h-3.5" />
            Changed Files ({review.changed_files.length})
          </div>
          <div className="flex flex-wrap gap-1.5">
            {review.changed_files.map((file) => (
              <Badge key={file} variant="outline" color="info" size="sm">
                {file}
              </Badge>
            ))}
          </div>
        </div>
      ) : null}

      {review.diff_preview ? (
        <div className="space-y-1.5">
          <div className="text-xs font-semibold uppercase tracking-wider text-slate-400">
            Unified Diff Preview
          </div>
          <CodeBlock language="diff">{review.diff_preview}</CodeBlock>
        </div>
      ) : null}
    </div>
  );
};

export const AiEmptyView: React.FC<{
  title: string;
  description: string;
  icon?: React.ReactNode;
  action?: React.ReactNode;
  actionLabel?: string;
  onAction?: () => void;
}> = ({ title, description, icon, action, actionLabel, onAction }) => {
  return (
    <div className="py-8 px-4 rounded-xl border border-dashed border-slate-700/60 bg-slate-900/20">
      <EmptyMessage fill="none">
        <EmptyMessage.Icon>
          {icon ?? <Sparkles className="w-6 h-6 text-blue-400" />}
        </EmptyMessage.Icon>
        <EmptyMessage.Title>{title}</EmptyMessage.Title>
        <EmptyMessage.Description>{description}</EmptyMessage.Description>
        {action ? (
          <EmptyMessage.ActionRow>{action}</EmptyMessage.ActionRow>
        ) : actionLabel && onAction ? (
          <EmptyMessage.ActionRow>
            <SdkButton color="primary" variant="solid" size="sm" onClick={onAction}>
              {actionLabel}
            </SdkButton>
          </EmptyMessage.ActionRow>
        ) : null}
      </EmptyMessage>
    </div>
  );
};

export const LogModeSwitch: React.FC<{
  mode?: "concise" | "detailed";
  value?: "concise" | "detailed";
  onChange: (mode: "concise" | "detailed") => void;
  conciseLabel?: string;
  detailedLabel?: string;
}> = ({
  mode,
  value,
  onChange,
  conciseLabel = "Concise Progress",
  detailedLabel = "Detailed Logs"
}) => {
  const current = value ?? mode ?? "concise";
  return (
    <SegmentedControl
      aria-label="Execution log verbosity"
      value={current}
      onChange={(next) => onChange(next as "concise" | "detailed")}
      size="sm"
      pill
    >
      <SegmentedControl.Option value="concise">
        {conciseLabel}
      </SegmentedControl.Option>
      <SegmentedControl.Option value="detailed">
        {detailedLabel}
      </SegmentedControl.Option>
    </SegmentedControl>
  );
};

export const HardwareTelemetryStrip: React.FC<{
  hardware?: HardwareSnapshot;
  cpuPercent?: number;
  memoryPercent?: number;
  availableMemoryMb?: number;
  onAcPower?: boolean;
  batteryPercent?: number | null;
  recommendedMaxConcurrency?: number;
}> = ({
  hardware,
  cpuPercent,
  memoryPercent,
  availableMemoryMb,
  onAcPower,
  batteryPercent,
  recommendedMaxConcurrency
}) => {
  const cpu = hardware?.cpu_usage_percent ?? cpuPercent ?? 0;
  const mem = hardware?.memory_usage_percent ?? memoryPercent ?? 0;
  const availMb = hardware?.available_memory_mb ?? availableMemoryMb ?? 0;
  const ac = hardware?.on_ac_power ?? onAcPower ?? true;
  const batt = hardware?.battery_percent ?? batteryPercent ?? null;
  const maxConc = hardware?.recommended_max_concurrency ?? recommendedMaxConcurrency ?? 2;

  return (
    <div className="flex flex-wrap items-center gap-2 text-xs">
      <Tooltip content="Host CPU utilization">
        <span className="inline-flex items-center gap-1 rounded-md border border-slate-700/60 bg-slate-900/60 px-2.5 py-1">
          <Cpu className="w-3.5 h-3.5 text-sky-400" />
          CPU {cpu.toFixed(0)}%
        </span>
      </Tooltip>
      <Tooltip content={`Available RAM: ${availMb} MB`}>
        <span className="inline-flex items-center gap-1 rounded-md border border-slate-700/60 bg-slate-900/60 px-2.5 py-1">
          <Info className="w-3.5 h-3.5 text-indigo-400" />
          RAM {mem.toFixed(0)}%
        </span>
      </Tooltip>
      <Badge
        variant="soft"
        color={ac ? "success" : "warning"}
        size="sm"
        pill
      >
        {ac
          ? `AC Power (Max ${maxConc} Agents)`
          : `Battery ${batt ?? ""}% (Max ${maxConc} Agent)`}
      </Badge>
    </div>
  );
};

export const PromptComposerBox: React.FC<{
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  onSubmit?: () => void;
  submitLabel?: string;
  disabled?: boolean;
}> = ({
  value,
  onChange,
  placeholder = "Describe task specification, acceptance criteria, and target files...",
  onSubmit,
  submitLabel = "Dispatch Task",
  disabled = false
}) => {
  return (
    <div className="space-y-2">
      <Textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        rows={4}
        autoResize
        maxRows={10}
        disabled={disabled}
      />
      {onSubmit ? (
        <div className="flex justify-end">
          <SdkButton
            color="primary"
            variant="solid"
            size="sm"
            onClick={onSubmit}
            disabled={disabled || !value.trim()}
          >
            {submitLabel}
          </SdkButton>
        </div>
      ) : null}
    </div>
  );
};
