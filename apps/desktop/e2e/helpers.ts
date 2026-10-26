import { expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

export const fixtureRepo = () => process.env.SYMPHONY_E2E_REPO!;

/** Direct runtime access for test setup (same discovery file the dev server uses). */
export function runtime(): { baseUrl: string; token: string } {
  const record = JSON.parse(readFileSync(join(process.env.SYMPHONY_E2E_DIR!, "data", "runtime.json"), "utf8")) as {
    base_url: string;
    token: string;
  };
  return { baseUrl: record.base_url, token: record.token };
}

export async function api<T>(method: string, path: string, body?: unknown): Promise<T> {
  const { baseUrl, token } = runtime();
  const res = await fetch(`${baseUrl}/api/v1${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body)
  });
  if (!res.ok) throw new Error(`${method} ${path} -> ${res.status} ${await res.text()}`);
  return (res.status === 204 ? undefined : await res.json()) as T;
}

/** Ensure onboarding is complete and preferences are deterministic before UI tests. */
export async function prepareRuntime(overrides: Record<string, unknown> = {}) {
  const settings = await api<Record<string, unknown>>("GET", "/settings");
  await api("PUT", "/settings", { ...settings, onboarding_completed: true, theme: "dark", locale: "en", ...overrides });
}

export async function openApp(
  page: Page,
  route = "/overview",
  opts: { theme?: "dark" | "light"; locale?: "en" | "sv"; /** `null`: first launch, the app picks the layout. */ inspectorOpen?: boolean | null } = {}
) {
  // Runtime settings are the persisted source of truth for theme/locale (the UI adopts them on
  // load), so set both the runtime and the local store.
  const settings = await api<Record<string, unknown>>("GET", "/settings");
  const theme = opts.theme ?? "dark";
  const locale = opts.locale ?? "en";
  if (settings.theme !== theme || settings.locale !== locale) {
    await api("PUT", "/settings", { ...settings, theme, locale });
  }
  await page.addInitScript(
    ([theme, locale, inspectorOpen]) => {
      try {
        const layout = inspectorOpen === null ? {} : { inspectorOpen };
        window.localStorage.setItem(
          "symphony.ui.v2",
          JSON.stringify({ state: { themeMode: theme, locale, sidebarCollapsed: false, ...layout, drawerOpen: true, lastRoute: "/overview" }, version: 1 })
        );
      } catch {
        /* ignore */
      }
    },
    [opts.theme ?? "dark", opts.locale ?? "en", opts.inspectorOpen === undefined ? true : opts.inspectorOpen] as const
  );
  await page.goto(`/#${route}`);
  await expect(page.getByRole("navigation", { name: opts.locale === "sv" ? "Huvudnavigering" : "Primary navigation" })).toBeVisible();
}
