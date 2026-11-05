import type { ModelService, ProfileStatus, ProviderProfile } from "@symphony/shared-types";

/**
 * The model services a connection profile can use. Mirrors `ModelService` in
 * `crates/symphony-core/src/connection.rs`; the runtime validates every profile again, so this is
 * only for early feedback and for presets.
 */
export interface ServiceInfo {
  id: ModelService;
  name: string;
  /** Vault key that holds the service's API key (in the credential allowlist). */
  credentialKey: string;
  baseUrl: string;
  host: string;
}

export const SERVICES: ReadonlyArray<ServiceInfo> = [
  {
    id: "openrouter",
    name: "OpenRouter",
    credentialKey: "OPENROUTER_API_KEY",
    baseUrl: "https://openrouter.ai/api/v1",
    host: "openrouter.ai"
  },
  {
    id: "opencode_go",
    name: "OpenCode Go",
    credentialKey: "OPENCODE_GO_API_KEY",
    baseUrl: "https://opencode.ai/inference/go",
    host: "opencode.ai"
  },
  {
    id: "nvidia_build",
    name: "NVIDIA Build",
    credentialKey: "NVIDIA_API_KEY",
    baseUrl: "https://integrate.api.nvidia.com/v1",
    host: "integrate.api.nvidia.com"
  }
];

/** Vault keys that belong to a service; other settings lists must not show them as plain integrations. */
export const SERVICE_CREDENTIAL_KEYS: ReadonlySet<string> = new Set(SERVICES.map((s) => s.credentialKey));

export function serviceInfo(id: ModelService | null | undefined): ServiceInfo | undefined {
  return SERVICES.find((s) => s.id === id);
}

export type BaseUrlProblem = "invalid" | "insecure" | "host";

/** `null` when empty (the preset is used) or acceptable; otherwise why it would be refused. */
export function baseUrlProblem(service: ServiceInfo, value: string): BaseUrlProblem | null {
  const text = value.trim();
  if (!text) return null;
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return "invalid";
  }
  if (url.protocol !== "https:") return "insecure";
  if (url.username || url.password || url.port || url.search || url.hash) return "invalid";
  return url.hostname.toLowerCase() === service.host ? null : "host";
}

/** "OpenRouter", then "OpenRouter 2", … not used by another profile. */
export function nextProfileName(service: ServiceInfo, existing: ReadonlyArray<ProviderProfile>): string {
  const taken = new Set(existing.map((p) => p.name));
  if (!taken.has(service.name)) return service.name;
  for (let n = 2; ; n += 1) {
    const name = `${service.name} ${n}`;
    if (!taken.has(name)) return name;
  }
}

export const MODEL_PATTERN = /^[A-Za-z0-9\-_./:@+]{1,200}$/;

export function modelProblem(value: string): boolean {
  const text = value.trim();
  return text.length > 0 && !MODEL_PATTERN.test(text);
}

export type StatusTone = "success" | "warning" | "danger" | "neutral";

export function statusTone(status: ProfileStatus): StatusTone {
  switch (status) {
    case "verified":
      return "success";
    case "limited":
      return "warning";
    case "error":
      return "danger";
    default:
      return "neutral";
  }
}
