import { asJsonObject, type AgentEvent, type JsonValue } from "@symphony/shared-types";

export type AgentEventKind =
  | "message"
  | "reasoning"
  | "command"
  | "file_change"
  | "tool_call"
  | "approval"
  | "error"
  | "session"
  | "lifecycle"
  | "other";

export type EventTone = "neutral" | "info" | "success" | "warning" | "danger" | "accent";

export interface FileChange {
  path: string;
  kind: string;
  diff: string | null;
}

export interface FormattedAgentEvent {
  kind: AgentEventKind;
  tone: EventTone;
  /** Short human summary (Markdown for agent messages, plain text otherwise). */
  text: string;
  command: string | null;
  output: string | null;
  exitCode: number | null;
  files: FileChange[];
  toolName: string | null;
}

type JsonObject = { [key: string]: JsonValue | undefined };
const asObject = asJsonObject;

function str(value: JsonValue | undefined): string | null {
  return typeof value === "string" ? value : null;
}

// CSI sequences (colours, cursor moves) that CLI agents write to their log streams.
// eslint-disable-next-line no-control-regex
const ANSI_ESCAPE = /\u001b\[[0-9;?]*[ -/]*[@-~]/g;

const plain = (value: string | null): string | null => (value === null ? null : value.replace(ANSI_ESCAPE, ""));

/**
 * Normalize runtime events (orchestrator lifecycle events and provider item events such as Codex
 * App Server `item/*` notifications) into a presentation model. Pure and memoizable per event id.
 * Terminal colour codes are removed from the free text, as the UI cannot render them.
 */
export function formatAgentEvent(event: AgentEvent): FormattedAgentEvent {
  const formatted = normalizeAgentEvent(event);
  return { ...formatted, text: plain(formatted.text) ?? "", command: plain(formatted.command), output: plain(formatted.output) };
}

function normalizeAgentEvent(event: AgentEvent): FormattedAgentEvent {
  const type = event.event_type;
  // Codex App Server events carry the raw JSON-RPC notification (`{ method, params: { item } }`);
  // orchestrator and simulated events carry the fields directly or under `item`.
  const raw = asObject(event.payload);
  const payload = asObject(raw?.params) ?? raw;
  const item = asObject(payload?.item) ?? payload;
  const itemType = str(item?.type ?? undefined) ?? "";
  const base: FormattedAgentEvent = {
    kind: "other",
    tone: "neutral",
    text: event.message,
    command: null,
    output: null,
    exitCode: null,
    files: [],
    toolName: null
  };

  if (type.includes("failed") || type.includes("error") || type === "agent/stderr" || event.phase === "failed") {
    return { ...base, kind: "error", tone: "danger" };
  }
  if (type.startsWith("approval/") || type.endsWith("requestApproval") || type.endsWith("approvalResolved")) {
    const command = str(payload?.command_or_diff) ?? str(payload?.command);
    return { ...base, kind: "approval", tone: type === "approval/requested" ? "warning" : "accent", command };
  }
  if (itemType === "agentMessage" || type === "item/agentMessage" || type.startsWith("claude/assistant")) {
    return { ...base, kind: "message", tone: "neutral", text: str(item?.text) ?? event.message };
  }
  if (itemType === "reasoning" || type.includes("reasoning")) {
    return { ...base, kind: "reasoning", tone: "neutral" };
  }
  if (itemType === "commandExecution" || type.startsWith("item/commandExecution")) {
    const exit = item?.exitCode;
    return {
      ...base,
      kind: "command",
      tone: typeof exit === "number" && exit !== 0 ? "danger" : "info",
      command: str(item?.command),
      output: str(item?.aggregatedOutput) || null,
      exitCode: typeof exit === "number" ? exit : null
    };
  }
  if (itemType === "fileChange" || type.startsWith("item/fileChange")) {
    const changes = Array.isArray(item?.changes) ? item.changes : [];
    const files = changes
      .map((c) => asObject(c))
      .filter((c): c is JsonObject => c !== null)
      .map((c) => ({
        path: str(c.path) ?? "?",
        // Codex reports `kind` as `{ type: "add" | "delete" | "update" }`.
        kind: str(c.kind) ?? str(asObject(c.kind)?.type) ?? "update",
        diff: str(c.diff)
      }));
    return { ...base, kind: "file_change", tone: "accent", files };
  }
  if (type === "item/tool/call" || itemType === "mcpToolCall" || itemType === "dynamicToolCall") {
    return { ...base, kind: "tool_call", tone: "info", toolName: str(item?.tool) ?? str(item?.name) };
  }
  if (type === "session_started" || type.startsWith("session") || type.startsWith("turn/")) {
    return { ...base, kind: "session", tone: type === "turn/completed" ? "success" : "neutral" };
  }
  if (
    type.startsWith("task/") ||
    type.startsWith("attempt/") ||
    type.startsWith("workspace/") ||
    type.startsWith("scheduler/") ||
    type.startsWith("project/") ||
    type.startsWith("pr/") ||
    type.startsWith("workflow/") ||
    type.startsWith("validation")
  ) {
    const tone: EventTone = type === "attempt/succeeded" || type === "pr/human_approved" ? "success" : "neutral";
    return { ...base, kind: "lifecycle", tone };
  }
  return base;
}

/** Event types an operator filter can toggle, grouped by kind. */
export const EVENT_KINDS: ReadonlyArray<AgentEventKind> = [
  "message",
  "reasoning",
  "command",
  "file_change",
  "tool_call",
  "approval",
  "error",
  "session",
  "lifecycle",
  "other"
];

/** Runtime events whose text the UI renders from its own EN/SV strings instead of `message`. */
export type RuntimeEventTemplate =
  | { key: "workflowReloaded" | "workflowSaved"; values: { path: string } }
  | { key: "workflowReloadFailed"; values: { error: string } };

/** The localized template for a runtime event, or `null` when the event's `message` is shown. */
export function runtimeEventTemplate(event: AgentEvent): RuntimeEventTemplate | null {
  const payload = asObject(event.payload);
  switch (event.event_type) {
    case "workflow/reloaded":
      return { key: "workflowReloaded", values: { path: str(payload?.path) ?? "" } };
    case "workflow/saved":
      return { key: "workflowSaved", values: { path: str(payload?.path) ?? "" } };
    case "workflow/reload_failed":
      return { key: "workflowReloadFailed", values: { error: str(payload?.error) ?? event.message } };
    default:
      return null;
  }
}
