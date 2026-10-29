/**
 * Performance evidence: initial load, view navigation latency, idle CPU, JS heap and long tasks while
 * simulated agents stream events. Results are written to docs/perf/perf-results.json.
 * These are measurements on the CI/dev machine running headless Chromium, not WebView2.
 */
import { expect, test, type CDPSession, type Page } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { api, fixtureRepo, openApp, prepareRuntime } from "./helpers.js";

const outDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../../docs/perf");

async function metrics(cdp: CDPSession): Promise<Record<string, number>> {
  const { metrics: list } = await cdp.send("Performance.getMetrics");
  return Object.fromEntries(list.map((m: { name: string; value: number }) => [m.name, m.value]));
}

async function installLongTaskObserver(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as { __longTasks: number[] };
    w.__longTasks = [];
    try {
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) w.__longTasks.push(e.duration);
      }).observe({ type: "longtask", buffered: true });
    } catch {
      /* unsupported */
    }
  });
}

test("collects performance measurements", async ({ page }) => {
  test.setTimeout(180_000);
  await prepareRuntime();
  const results: Record<string, unknown> = { measured_at: new Date().toISOString(), environment: "headless Chromium (Playwright), Vite dev server, simulated agent" };
  await installLongTaskObserver(page);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");

  const t0 = Date.now();
  await openApp(page, "/overview");
  await expect(page.getByRole("contentinfo")).toContainText("Runtime: Live");
  results.initial_load_to_live_ms = Date.now() - t0;
  const nav = await page.evaluate(() => {
    const e = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming;
    return { dom_content_loaded_ms: Math.round(e.domContentLoadedEventEnd), load_ms: Math.round(e.loadEventEnd) };
  });
  results.navigation_timing = nav;

  // View switching latency (client-side routing, includes lazy chunk load on first visit).
  const views = ["Projects", "Tasks", "Worktrees", "Pull requests", "Workflow Studio", "Providers", "Settings", "Overview"];
  const navLatency: Record<string, number> = {};
  for (const label of views) {
    const start = Date.now();
    await page.getByRole("navigation", { name: "Primary navigation" }).getByRole("link", { name: label }).click();
    await expect(page.locator("main h1").first()).toBeVisible();
    navLatency[label] = Date.now() - start;
  }
  results.first_visit_navigation_ms = navLatency;

  // Idle CPU: script + layout time over 10 s with the stream connected and nothing running.
  const idleBefore = await metrics(cdp);
  await page.waitForTimeout(10_000);
  const idleAfter = await metrics(cdp);
  results.idle_10s = {
    script_ms: Math.round(((idleAfter.ScriptDuration ?? 0) - (idleBefore.ScriptDuration ?? 0)) * 1000),
    layout_ms: Math.round(((idleAfter.LayoutDuration ?? 0) - (idleBefore.LayoutDuration ?? 0)) * 1000),
    task_ms: Math.round(((idleAfter.TaskDuration ?? 0) - (idleBefore.TaskDuration ?? 0)) * 1000),
    js_heap_used_mb: Math.round(((idleAfter.JSHeapUsedSize ?? 0) / 1024 / 1024) * 10) / 10
  };

  // Streaming load: three simulated agents running concurrently while the operator watches the drawer.
  const projects = await api<{ id: string; repo_path: string }[]>("GET", "/projects");
  const project =
    projects.find((p) => p.repo_path === fixtureRepo()) ??
    (await api<{ id: string }>("POST", "/projects", { name: "perf-fixture", repo_path: fixtureRepo(), preferred_provider: "simulated" }));
  await page.evaluate(() => ((window as unknown as { __longTasks: number[] }).__longTasks = []));
  const streamBefore = await metrics(cdp);
  const started = Date.now();
  for (let i = 0; i < 3; i++) {
    await api("POST", "/tasks", { project_id: project.id, title: `perf run ${i}`, provider: "simulated", auto_start: true });
  }
  await expect.poll(async () => (await api<{ counts: { running: number } }>("GET", "/state")).counts.running, { timeout: 60_000 }).toBe(0);
  const streamAfter = await metrics(cdp);
  const longTasks = await page.evaluate(() => (window as unknown as { __longTasks: number[] }).__longTasks);
  results.streaming_3_agents = {
    wall_ms: Date.now() - started,
    script_ms: Math.round(((streamAfter.ScriptDuration ?? 0) - (streamBefore.ScriptDuration ?? 0)) * 1000),
    long_tasks: longTasks.length,
    longest_task_ms: Math.round(Math.max(0, ...longTasks)),
    js_heap_used_mb: Math.round(((streamAfter.JSHeapUsedSize ?? 0) / 1024 / 1024) * 10) / 10,
    dom_nodes: streamAfter.Nodes
  };

  mkdirSync(outDir, { recursive: true });
  writeFileSync(resolve(outDir, "perf-results.json"), JSON.stringify(results, null, 2) + "\n");
  console.log(JSON.stringify(results, null, 2));
});
