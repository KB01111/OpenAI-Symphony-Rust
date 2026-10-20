import { describe, expect, it } from "vitest";
import {
  STORAGE_KEYS,
  classifyConnectionError,
  forgetToken,
  loadConnection,
  loadLocale,
  normalizeBaseUrl,
  saveConnection,
  type KeyValueStorage,
  type StorageBackends
} from "../src/connection.js";

function memory(): KeyValueStorage & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k)
  };
}

function backends() {
  const local = memory();
  const session = memory();
  return { local, session, all: { local, session } as StorageBackends };
}

describe("normalizeBaseUrl", () => {
  it("defaults to http, strips trailing slashes, query and a pasted /api/v1", () => {
    expect(normalizeBaseUrl(" 192.168.1.20:47821/ ")).toEqual({ ok: true, url: "http://192.168.1.20:47821" });
    expect(normalizeBaseUrl("https://host.local/api/v1/?x=1#y")).toEqual({ ok: true, url: "https://host.local" });
    expect(normalizeBaseUrl("https://host.local/prefix/")).toEqual({ ok: true, url: "https://host.local/prefix" });
  });

  it("rejects empty, non-http and credential-bearing URLs", () => {
    expect(normalizeBaseUrl("  ")).toEqual({ ok: false, problem: "empty" });
    expect(normalizeBaseUrl("javascript:alert(1)")).toEqual({ ok: false, problem: "unsupported_scheme" });
    expect(normalizeBaseUrl("ftp://host")).toEqual({ ok: false, problem: "unsupported_scheme" });
    expect(normalizeBaseUrl("http://user:pw@host")).toEqual({ ok: false, problem: "invalid" });
  });
});

describe("token storage policy", () => {
  it("keeps the token in sessionStorage unless the user opts in", () => {
    const b = backends();
    saveConnection(b.all, { baseUrl: "http://h:1", token: "secret", remember: false });
    expect(b.session.data.get(STORAGE_KEYS.token)).toBe("secret");
    expect(b.local.data.has(STORAGE_KEYS.token)).toBe(false);
    expect(loadConnection(b.all)).toEqual({ baseUrl: "http://h:1", token: "secret", remember: false });
  });

  it("stores the token in localStorage only when remembered, and forgets it everywhere", () => {
    const b = backends();
    saveConnection(b.all, { baseUrl: "http://h:1", token: "secret", remember: true });
    expect(b.local.data.get(STORAGE_KEYS.token)).toBe("secret");
    expect(b.session.data.has(STORAGE_KEYS.token)).toBe(false);
    forgetToken(b.all);
    expect(loadConnection(b.all)).toEqual({ baseUrl: "http://h:1", token: "", remember: false });
  });

  it("removes a legacy localStorage token that was stored without consent", () => {
    const b = backends();
    b.local.data.set(STORAGE_KEYS.legacyHost, "10.0.0.2:4780");
    b.local.data.set(STORAGE_KEYS.token, "old");
    const loaded = loadConnection(b.all);
    expect(loaded).toEqual({ baseUrl: "http://10.0.0.2:4780", token: "", remember: false });
    expect(b.local.data.has(STORAGE_KEYS.token)).toBe(false);
    expect(b.local.data.has(STORAGE_KEYS.legacyHost)).toBe(false);
  });

  it("tolerates unavailable or throwing storage", () => {
    const throwing: KeyValueStorage = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
      removeItem: () => {
        throw new Error("blocked");
      }
    };
    expect(() => saveConnection({ local: throwing, session: null }, { baseUrl: "http://h", token: "t", remember: true })).not.toThrow();
    expect(loadConnection({ local: throwing, session: null })).toBeNull();
  });

  it("picks the saved locale, else the browser language", () => {
    const b = backends();
    expect(loadLocale(b.all, "sv-SE")).toBe("sv");
    expect(loadLocale(b.all, "en-US")).toBe("en");
    b.local.data.set(STORAGE_KEYS.locale, "en");
    expect(loadLocale(b.all, "sv-SE")).toBe("en");
  });
});

describe("classifyConnectionError", () => {
  it("maps statuses and network failures", () => {
    expect(classifyConnectionError({ kind: "unauthorized", status: 401 })).toBe("unauthorized");
    expect(classifyConnectionError({ status: 403 })).toBe("unauthorized");
    expect(classifyConnectionError({ kind: "network", status: 0 })).toBe("network");
    expect(classifyConnectionError(new TypeError("Failed to fetch"))).toBe("network");
    expect(classifyConnectionError(new Error("boom"))).toBe("other");
  });
});
