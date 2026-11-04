/**
 * Wire protocol between Symphony (`crates/symphony-agents/src/codesandbox.rs`) and this bridge:
 * one JSON request line on stdin, JSON event lines on stdout. stdout carries nothing else.
 */

export const CLOUD_AGENTS = ["mini_swe_agent", "pi", "pi_durable"] as const;
export type CloudAgent = (typeof CLOUD_AGENTS)[number];

export const VM_TIERS = ["pico", "nano", "micro", "small", "medium", "large", "xlarge"] as const;
export type VmTierName = (typeof VM_TIERS)[number];

export interface GitCredentials {
  token: string;
  name: string;
  email: string;
}

/** Run one agent turn in a sandbox (created from the template, or resumed from `sandboxId`). */
export interface TurnRequest {
  type: "turn";
  csbApiKey: string;
  templateId: string;
  sandboxId?: string | null;
  taskId: string;
  issueIdentifier: string;
  turnId: string;
  prompt: string;
  agent: CloudAgent;
  model?: string | null;
  repoUrl: string;
  branch: string;
  baseBranch: string;
  vmTier: VmTierName;
  hibernationTimeoutSecs: number;
  costLimitUsd: number;
  turnTimeoutMs: number;
  git: GitCredentials;
  /** Model provider keys etc. for the agent process. */
  env: Record<string, string>;
}

/** Act on an existing sandbox without running a turn. */
export interface ControlRequest {
  type: "abort" | "stop";
  csbApiKey: string;
  sandboxId: string;
  /** `stop` only: hibernate (default, keeps files and the agent checkpoint) or shut down. */
  mode?: "hibernate" | "shutdown" | null;
}

export type BridgeRequest = TurnRequest | ControlRequest;

export type BridgeEvent =
  | { type: "started"; sandboxId: string; resumed: boolean }
  | { type: "output"; text: string }
  | {
      type: "tool";
      callId: string;
      name: string;
      kind: "command" | "file_change" | "mcp" | "provider_tool";
      status: "started" | "succeeded" | "failed";
      input?: unknown;
      output?: string;
      durationMs?: number;
    }
  | { type: "usage"; inputTokens: number; outputTokens: number }
  | { type: "pushed"; sha: string; branch: string }
  | { type: "turn_completed"; succeeded: boolean; errorCode?: string; message?: string }
  | { type: "error"; code: string; message: string };

/** Prefix of the structured lines the in-sandbox drivers print; every other line is plain output. */
export const DRIVER_EVENT_PREFIX = "@@SYMPHONY ";

export class RequestError extends Error {}

function str(value: unknown, field: string, { empty = false } = {}): string {
  if (typeof value !== "string" || (!empty && value.trim() === "")) {
    throw new RequestError(`${field} must be a non-empty string`);
  }
  return value;
}

function num(value: unknown, field: string, fallback: number): number {
  if (value === undefined || value === null) return fallback;
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new RequestError(`${field} must be a non-negative number`);
  }
  return value;
}

/**
 * Parse and validate a request line. Errors name the field only, never its value: the line holds
 * credentials.
 */
export function parseRequest(line: string): BridgeRequest {
  let raw: unknown;
  try {
    raw = JSON.parse(line);
  } catch {
    throw new RequestError("the request is not valid JSON");
  }
  if (typeof raw !== "object" || raw === null) throw new RequestError("the request must be an object");
  const r = raw as Record<string, unknown>;

  if (r.type === "abort" || r.type === "stop") {
    const mode = r.mode ?? null;
    if (mode !== null && mode !== "hibernate" && mode !== "shutdown") {
      throw new RequestError("mode must be hibernate or shutdown");
    }
    return {
      type: r.type,
      csbApiKey: str(r.csbApiKey, "csbApiKey"),
      sandboxId: str(r.sandboxId, "sandboxId"),
      mode,
    };
  }
  if (r.type !== "turn") throw new RequestError("type must be turn, abort or stop");

  const agent = str(r.agent, "agent");
  if (!(CLOUD_AGENTS as readonly string[]).includes(agent)) {
    throw new RequestError(`agent must be one of ${CLOUD_AGENTS.join(", ")}`);
  }
  const vmTier = str(r.vmTier ?? "small", "vmTier").toLowerCase();
  if (!(VM_TIERS as readonly string[]).includes(vmTier)) {
    throw new RequestError(`vmTier must be one of ${VM_TIERS.join(", ")}`);
  }
  const repoUrl = str(r.repoUrl, "repoUrl");
  if (!/^https:\/\/[^\s@]+$/.test(repoUrl)) {
    // No userinfo: the token is supplied as a git credential, never inside the URL.
    throw new RequestError("repoUrl must be an https:// URL without credentials");
  }
  const branch = str(r.branch, "branch");
  if (branch.startsWith("-") || /\s/.test(branch)) throw new RequestError("branch is not a valid branch name");
  const git = (r.git ?? {}) as Record<string, unknown>;
  const env: Record<string, string> = {};
  if (r.env !== undefined && r.env !== null) {
    if (typeof r.env !== "object") throw new RequestError("env must be an object");
    for (const [key, value] of Object.entries(r.env as Record<string, unknown>)) {
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key) || typeof value !== "string") {
        throw new RequestError("env holds an invalid entry");
      }
      env[key] = value;
    }
  }
  return {
    type: "turn",
    csbApiKey: str(r.csbApiKey, "csbApiKey"),
    templateId: str(r.templateId, "templateId"),
    sandboxId: typeof r.sandboxId === "string" && r.sandboxId ? r.sandboxId : null,
    taskId: str(r.taskId, "taskId"),
    issueIdentifier: str(r.issueIdentifier, "issueIdentifier"),
    turnId: str(r.turnId, "turnId"),
    prompt: str(r.prompt, "prompt"),
    agent: agent as CloudAgent,
    model: typeof r.model === "string" && r.model.trim() ? r.model.trim() : null,
    repoUrl,
    branch,
    baseBranch: str(r.baseBranch ?? "main", "baseBranch"),
    vmTier: vmTier as VmTierName,
    hibernationTimeoutSecs: num(r.hibernationTimeoutSecs, "hibernationTimeoutSecs", 1800),
    costLimitUsd: num(r.costLimitUsd, "costLimitUsd", 0),
    turnTimeoutMs: num(r.turnTimeoutMs, "turnTimeoutMs", 3_600_000),
    git: {
      token: str(git.token, "git.token"),
      name: str(git.name, "git.name"),
      email: str(git.email, "git.email"),
    },
    env,
  };
}

/** Secrets of a request, longest first, for masking in anything the bridge prints. */
export function secretsOf(request: BridgeRequest): string[] {
  const values = [request.csbApiKey];
  if (request.type === "turn") {
    values.push(request.git.token, ...Object.values(request.env));
  }
  return values
    .filter((v) => v.length >= 6)
    .sort((a, b) => b.length - a.length);
}

export function redact(text: string, secrets: readonly string[]): string {
  let out = text;
  for (const secret of secrets) out = out.split(secret).join("[REDACTED]");
  // Same shapes as symphony-core's redaction: tokens that are not (yet) in the request.
  return out
    .replace(/gh[pousr]_[A-Za-z0-9_]{16,255}/g, "[REDACTED]")
    .replace(/github_pat_[A-Za-z0-9_]{16,255}/g, "[REDACTED]")
    .replace(/csb_v\d+_[A-Za-z0-9_-]{16,255}/g, "[REDACTED]")
    .replace(/sk-[A-Za-z0-9_-]{16,255}/g, "[REDACTED]");
}

/**
 * Parse a driver line. A line with {@link DRIVER_EVENT_PREFIX} and valid JSON is a structured
 * event; anything else is plain agent output.
 */
export function parseDriverLine(line: string): BridgeEvent {
  if (line.startsWith(DRIVER_EVENT_PREFIX)) {
    try {
      const event = JSON.parse(line.slice(DRIVER_EVENT_PREFIX.length)) as BridgeEvent;
      if (event && typeof event === "object" && ["output", "tool", "usage", "pushed"].includes(event.type)) {
        // Any process in the sandbox can print a line with this prefix. A `pushed` event makes the host
        // update the worktree, so it must carry what a push really reports: a hex commit id.
        if (event.type === "pushed" && !(typeof event.sha === "string" && /^[0-9a-f]{7,64}$/i.test(event.sha))) {
          return { type: "output", text: line };
        }
        return event;
      }
    } catch {
      // fall through: show it as output
    }
  }
  return { type: "output", text: line };
}
