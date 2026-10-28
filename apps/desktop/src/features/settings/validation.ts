import { formatMessage, type AppSettings } from "@symphony/shared-types";
import type { SettingsMessages } from "./messages.js";

export type SettingsField =
  | "polling_interval_ms"
  | "event_retention_days"
  | "backup_interval_hours"
  | "backup_keep"
  | "pr_refresh_interval_minutes"
  | "max_concurrent_on_ac"
  | "max_concurrent_on_battery"
  | "high_memory_threshold_percent"
  | "high_cpu_threshold_percent"
  | "remote_bind_address"
  | "remote_port"
  | "remote_allowed_origins";

export type SettingsErrors = Partial<Record<SettingsField, string>>;

function isInt(n: number): boolean {
  return Number.isFinite(n) && Number.isInteger(n);
}

/**
 * Client-side mirror of `validate_settings` in `crates/symphony-server/src/lib.rs`. The runtime
 * re-validates every save and remains authoritative; this only blocks obviously invalid input.
 */
export function validateSettings(s: AppSettings, v: SettingsMessages["validation"], localApiPort?: number | null): SettingsErrors {
  const errors: SettingsErrors = {};
  const p = s.resource_policy;

  if (!isInt(s.polling_interval_ms)) errors.polling_interval_ms = v.integer;
  else if (s.polling_interval_ms < 2000) errors.polling_interval_ms = v.polling;

  const retention = s.event_retention_days ?? 30;
  if (!isInt(retention)) errors.event_retention_days = v.integer;
  else if (retention < 0 || retention > 3650) errors.event_retention_days = v.retention;

  const backupInterval = s.backup_interval_hours ?? 24;
  if (!isInt(backupInterval)) errors.backup_interval_hours = v.integer;
  else if (backupInterval < 0 || backupInterval > 720) errors.backup_interval_hours = v.backupInterval;

  const backupKeep = s.backup_keep ?? 7;
  if (!isInt(backupKeep)) errors.backup_keep = v.integer;
  else if (backupKeep < 1 || backupKeep > 100) errors.backup_keep = v.backupKeep;

  const prRefresh = s.pr_refresh_interval_minutes ?? 15;
  if (!isInt(prRefresh)) errors.pr_refresh_interval_minutes = v.integer;
  else if (prRefresh !== 0 && (prRefresh < 5 || prRefresh > 1440)) errors.pr_refresh_interval_minutes = v.prRefresh;

  if (!isInt(p.max_concurrent_on_ac)) errors.max_concurrent_on_ac = v.integer;
  else if (p.max_concurrent_on_ac < 1 || p.max_concurrent_on_ac > 32) errors.max_concurrent_on_ac = v.maxAc;

  if (!isInt(p.max_concurrent_on_battery)) errors.max_concurrent_on_battery = v.integer;
  else if (p.max_concurrent_on_battery < 0 || (isInt(p.max_concurrent_on_ac) && p.max_concurrent_on_battery > p.max_concurrent_on_ac)) {
    errors.max_concurrent_on_battery = formatMessage(v.maxBattery, { max: p.max_concurrent_on_ac });
  }

  for (const key of ["high_memory_threshold_percent", "high_cpu_threshold_percent"] as const) {
    const value = p[key];
    if (!Number.isFinite(value)) errors[key] = v.number;
    else if (value < 10 || value > 100) errors[key] = v.threshold;
  }

  if (!s.remote_bind_address.trim()) errors.remote_bind_address = v.bind;
  // The runtime only binds the remote listener to loopback; exposure goes through a TLS proxy.
  else if (!isLoopback(s.remote_bind_address)) errors.remote_bind_address = v.bindLoopback;
  const badOrigin = (s.remote_allowed_origins ?? []).find((origin) => !isValidRemoteOrigin(origin));
  if (badOrigin !== undefined) errors.remote_allowed_origins = formatMessage(v.origin, { origin: badOrigin });
  if (!isInt(s.remote_port)) errors.remote_port = v.integer;
  else if (s.remote_port < 1 || s.remote_port > 65535) errors.remote_port = v.port;
  // The desktop host refuses to start the remote listener on the local API's port.
  else if (localApiPort && s.remote_port === localApiPort) errors.remote_port = formatMessage(v.portInUse, { port: localApiPort });

  return errors;
}

/** Map a runtime `invalid_settings` message to the field it is about, so it can be shown inline. */
export function serverErrorFields(message: string): SettingsField[] {
  if (message.includes("max_concurrent_on_battery")) return ["max_concurrent_on_battery"];
  if (message.includes("max_concurrent_on_ac")) return ["max_concurrent_on_ac"];
  if (message.includes("polling_interval_ms")) return ["polling_interval_ms"];
  if (message.includes("event_retention_days")) return ["event_retention_days"];
  if (message.includes("backup_interval_hours")) return ["backup_interval_hours"];
  if (message.includes("backup_keep")) return ["backup_keep"];
  if (message.includes("pr_refresh_interval_minutes")) return ["pr_refresh_interval_minutes"];
  if (message.includes("remote_port")) return ["remote_port"];
  if (message.includes("remote_bind_address")) return ["remote_bind_address"];
  if (message.includes("origin")) return ["remote_allowed_origins"];
  if (message.includes("thresholds")) return ["high_memory_threshold_percent", "high_cpu_threshold_percent"];
  return [];
}

/** Bind addresses that keep the remote listener on this machine. */
export function isLoopback(address: string): boolean {
  const a = address.trim().toLowerCase();
  const v4 = /^127\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(a);
  return a === "localhost" || a === "::1" || a === "[::1]" || (v4 !== null && v4.slice(1).every((octet) => Number(octet) <= 255));
}

/**
 * Client-side mirror of `validate_remote_origin` (`crates/symphony-server/src/remote.rs`): a bare
 * `https://` origin, `http://` only for loopback hosts, or the companion app's `tauri://localhost`.
 */
export function isValidRemoteOrigin(origin: string): boolean {
  if (origin === "tauri://localhost") return true;
  if (origin.includes("*")) return false;
  let url: URL;
  try {
    url = new URL(origin);
  } catch {
    return false;
  }
  if (url.origin !== origin) return false;
  if (url.protocol === "https:") return true;
  return url.protocol === "http:" && isLoopback(url.hostname);
}

/** One origin per line (or comma separated), blanks dropped. */
export function parseOrigins(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((o) => o.trim())
    .filter((o) => o.length > 0);
}

export function formatBytes(bytes: number, locale: string): string {
  const fmt = (n: number, unit: string) =>
    `${new Intl.NumberFormat(locale, { maximumFractionDigits: n < 10 ? 1 : 0 }).format(n)} ${unit}`;
  if (bytes < 1024) return fmt(bytes, "B");
  if (bytes < 1024 * 1024) return fmt(bytes / 1024, "KB");
  return fmt(bytes / (1024 * 1024), "MB");
}

/** Port of the runtime's local API from its base URL (`http://127.0.0.1:4780`), if any. */
export function portOf(baseUrl: string | null | undefined): number | null {
  if (!baseUrl) return null;
  try {
    const url = new URL(baseUrl);
    return url.port ? Number(url.port) : null;
  } catch {
    return null;
  }
}
