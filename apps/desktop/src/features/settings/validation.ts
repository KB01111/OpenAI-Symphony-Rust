import { formatMessage, type AppSettings } from "@symphony/shared-types";
import type { SettingsMessages } from "./messages.js";

export type SettingsField =
  | "polling_interval_ms"
  | "max_concurrent_on_ac"
  | "max_concurrent_on_battery"
  | "high_memory_threshold_percent"
  | "high_cpu_threshold_percent"
  | "remote_bind_address"
  | "remote_port";

export type SettingsErrors = Partial<Record<SettingsField, string>>;

function isInt(n: number): boolean {
  return Number.isFinite(n) && Number.isInteger(n);
}

/**
 * Client-side mirror of `validate_settings` in `crates/symphony-server/src/lib.rs`. The runtime
 * re-validates every save and remains authoritative; this only blocks obviously invalid input.
 */
export function validateSettings(s: AppSettings, v: SettingsMessages["validation"]): SettingsErrors {
  const errors: SettingsErrors = {};
  const p = s.resource_policy;

  if (!isInt(s.polling_interval_ms)) errors.polling_interval_ms = v.integer;
  else if (s.polling_interval_ms < 2000) errors.polling_interval_ms = v.polling;

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
  if (!isInt(s.remote_port)) errors.remote_port = v.integer;
  else if (s.remote_port < 1 || s.remote_port > 65535) errors.remote_port = v.port;

  return errors;
}

/** Map a runtime `invalid_settings` message to the field it is about, so it can be shown inline. */
export function serverErrorFields(message: string): SettingsField[] {
  if (message.includes("max_concurrent_on_battery")) return ["max_concurrent_on_battery"];
  if (message.includes("max_concurrent_on_ac")) return ["max_concurrent_on_ac"];
  if (message.includes("polling_interval_ms")) return ["polling_interval_ms"];
  if (message.includes("remote_port")) return ["remote_port"];
  if (message.includes("thresholds")) return ["high_memory_threshold_percent", "high_cpu_threshold_percent"];
  return [];
}

/** Bind addresses that keep the remote listener on this machine. */
export function isLoopback(address: string): boolean {
  const a = address.trim().toLowerCase();
  return a === "localhost" || a === "::1" || a === "[::1]" || a.startsWith("127.");
}

export function formatBytes(bytes: number, locale: string): string {
  const fmt = (n: number, unit: string) =>
    `${new Intl.NumberFormat(locale, { maximumFractionDigits: n < 10 ? 1 : 0 }).format(n)} ${unit}`;
  if (bytes < 1024) return fmt(bytes, "B");
  if (bytes < 1024 * 1024) return fmt(bytes / 1024, "KB");
  return fmt(bytes / (1024 * 1024), "MB");
}
