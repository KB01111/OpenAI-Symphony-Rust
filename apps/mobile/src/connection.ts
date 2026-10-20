/**
 * Connection settings for the remote companion: runtime URL normalisation, token storage policy
 * and error classification. Pure logic (no React, no runtime imports from workspace packages) so
 * it can be unit tested with `node --test`.
 *
 * Storage policy:
 * - the runtime URL is not secret and is kept in `localStorage` for convenience;
 * - the access token is kept in `sessionStorage` (cleared when the tab/app session ends) unless
 *   the user explicitly ticks "remember on this device", in which case it goes to `localStorage`;
 * - storage can be unavailable (private mode, blocked site data) — every access is guarded.
 */
import type { Locale } from "@symphony/shared-types";

export interface ConnectionSettings {
  baseUrl: string;
  token: string;
  remember: boolean;
}

/** Minimal Storage surface (lets tests inject in-memory stores). */
export interface KeyValueStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface StorageBackends {
  local: KeyValueStorage | null;
  session: KeyValueStorage | null;
}

export const STORAGE_KEYS = {
  baseUrl: "symphony.mobile.baseUrl",
  token: "symphony.mobile.token",
  remember: "symphony.mobile.rememberToken",
  locale: "symphony.mobile.locale",
  /** Pre-2.0 keys: the old companion stored the token in localStorage unconditionally. */
  legacyHost: "symphony.mobile.host"
} as const;

function safeGet(storage: KeyValueStorage | null, key: string): string | null {
  if (!storage) return null;
  try {
    return storage.getItem(key);
  } catch {
    return null;
  }
}

function safeSet(storage: KeyValueStorage | null, key: string, value: string): boolean {
  if (!storage) return false;
  try {
    storage.setItem(key, value);
    return true;
  } catch {
    return false;
  }
}

function safeRemove(storage: KeyValueStorage | null, key: string): void {
  if (!storage) return;
  try {
    storage.removeItem(key);
  } catch {
    /* storage unavailable */
  }
}

/** Resolve the browser storages, tolerating environments where accessing them throws. */
export function browserStorage(): StorageBackends {
  const pick = (name: "localStorage" | "sessionStorage"): KeyValueStorage | null => {
    try {
      const storage = (globalThis as unknown as Record<string, KeyValueStorage | undefined>)[name];
      return storage ?? null;
    } catch {
      return null;
    }
  };
  return { local: pick("localStorage"), session: pick("sessionStorage") };
}

export type UrlProblem = "empty" | "invalid" | "unsupported_scheme" | "insecure_scheme";

/** Hosts on this device, where plain `http://` never leaves the machine. */
export function isLoopbackHost(hostname: string): boolean {
  const h = hostname.toLowerCase();
  return h === "localhost" || h === "[::1]" || h === "::1" || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(h);
}

export type NormalizedUrl = { ok: true; url: string } | { ok: false; problem: UrlProblem };

/**
 * Normalise user input into the runtime base URL (`scheme://host[:port][/prefix]`):
 * trims whitespace, defaults to `https://` when no scheme is given (`http://` for loopback hosts),
 * rejects non-HTTP schemes and plain `http://` to any other host — the bearer token must never
 * cross a network unencrypted — and drops query/fragment, trailing slashes and a pasted `/api/v1`
 * suffix (the client adds it).
 */
export function normalizeBaseUrl(input: string): NormalizedUrl {
  const trimmed = input.trim();
  if (!trimmed) return { ok: false, problem: "empty" };
  const hasScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed);
  if (!hasScheme && /^[a-z][a-z0-9+.-]*:(?!\d)/i.test(trimmed)) {
    // e.g. "javascript:alert(1)" or "file:/x" — a scheme without `//`.
    return { ok: false, problem: "unsupported_scheme" };
  }
  let parsed: URL;
  try {
    parsed = new URL(hasScheme ? trimmed : `https://${trimmed}`);
    if (!hasScheme && isLoopbackHost(parsed.hostname)) parsed = new URL(`http://${trimmed}`);
  } catch {
    return { ok: false, problem: "invalid" };
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return { ok: false, problem: "unsupported_scheme" };
  }
  if (!parsed.hostname) return { ok: false, problem: "invalid" };
  if (parsed.protocol === "http:" && !isLoopbackHost(parsed.hostname)) {
    return { ok: false, problem: "insecure_scheme" };
  }
  if (parsed.username || parsed.password) return { ok: false, problem: "invalid" };
  let path = parsed.pathname.replace(/\/+$/, "");
  path = path.replace(/\/api\/v1$/i, "").replace(/\/+$/, "");
  return { ok: true, url: `${parsed.protocol}//${parsed.host}${path}` };
}

/** Load the saved connection. Returns `null` when no runtime URL has been saved. */
export function loadConnection(backends: StorageBackends): ConnectionSettings | null {
  const { local, session } = backends;
  let baseUrl = safeGet(local, STORAGE_KEYS.baseUrl);
  const legacyHost = safeGet(local, STORAGE_KEYS.legacyHost);
  if (!baseUrl && legacyHost) {
    const migrated = normalizeBaseUrl(legacyHost);
    if (migrated.ok) baseUrl = migrated.url;
  }
  if (legacyHost !== null) safeRemove(local, STORAGE_KEYS.legacyHost);
  if (!baseUrl) return null;
  // A URL saved by an older companion may be plain http:// to another device; never reuse it,
  // and do not leave the token that went with it on the device either.
  if (!normalizeBaseUrl(baseUrl).ok) {
    forgetToken(backends);
    safeRemove(local, STORAGE_KEYS.baseUrl);
    return null;
  }

  const remember = safeGet(local, STORAGE_KEYS.remember) === "true";
  const token = remember ? safeGet(local, STORAGE_KEYS.token) : safeGet(session, STORAGE_KEYS.token);
  if (!remember) {
    // A token in localStorage without the explicit opt-in (e.g. written by the pre-2.0 companion)
    // must not linger on the device.
    safeRemove(local, STORAGE_KEYS.token);
  }
  return { baseUrl, token: token ?? "", remember };
}

/** Persist a connection according to the storage policy above. */
export function saveConnection(backends: StorageBackends, settings: ConnectionSettings): void {
  const { local, session } = backends;
  safeSet(local, STORAGE_KEYS.baseUrl, settings.baseUrl);
  if (settings.remember) {
    safeSet(local, STORAGE_KEYS.remember, "true");
    if (settings.token) safeSet(local, STORAGE_KEYS.token, settings.token);
    else safeRemove(local, STORAGE_KEYS.token);
    safeRemove(session, STORAGE_KEYS.token);
  } else {
    safeRemove(local, STORAGE_KEYS.remember);
    safeRemove(local, STORAGE_KEYS.token);
    if (settings.token) safeSet(session, STORAGE_KEYS.token, settings.token);
    else safeRemove(session, STORAGE_KEYS.token);
  }
}

/** Forget the token everywhere (the runtime URL is kept to make reconnecting easy). */
export function forgetToken(backends: StorageBackends): void {
  safeRemove(backends.local, STORAGE_KEYS.token);
  safeRemove(backends.local, STORAGE_KEYS.remember);
  safeRemove(backends.session, STORAGE_KEYS.token);
}

export function loadLocale(backends: StorageBackends, browserLanguage?: string | null): Locale {
  const saved = safeGet(backends.local, STORAGE_KEYS.locale);
  if (saved === "en" || saved === "sv") return saved;
  return browserLanguage?.toLowerCase().startsWith("sv") ? "sv" : "en";
}

export function saveLocale(backends: StorageBackends, locale: Locale): void {
  safeSet(backends.local, STORAGE_KEYS.locale, locale);
}

export type ConnectionErrorKind = "unauthorized" | "network" | "other";

/**
 * Classify a request failure. Structural so it works with `SymphonyApiError` (`kind`/`status`)
 * without importing the client at runtime.
 */
export function classifyConnectionError(error: unknown): ConnectionErrorKind {
  if (error && typeof error === "object") {
    const { kind, status } = error as { kind?: unknown; status?: unknown };
    if (kind === "unauthorized" || status === 401 || status === 403) return "unauthorized";
    if (kind === "network" || status === 0) return "network";
  }
  if (error instanceof TypeError) return "network";
  return "other";
}
