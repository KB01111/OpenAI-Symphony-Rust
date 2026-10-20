import { expect, test } from "@playwright/test";
import { openApp, prepareRuntime } from "./helpers.js";

test.beforeEach(async () => {
  await prepareRuntime();
});

test("navigates every primary view without full page reloads", async ({ page }) => {
  await openApp(page, "/overview");
  const marker = await page.evaluate(() => ((window as unknown as { __navMarker: number }).__navMarker = Math.random()));
  const views: [string, RegExp][] = [
    ["Projects", /#\/projects/],
    ["Tasks", /#\/tasks/],
    ["Agent runs", /#\/runs/],
    ["Worktrees", /#\/worktrees/],
    ["Pull requests", /#\/reviews/],
    ["Workflow Studio", /#\/workflow/],
    ["Providers", /#\/providers/],
    ["Settings", /#\/settings/],
    ["Overview", /#\/overview/]
  ];
  for (const [label, url] of views) {
    await page.getByRole("navigation", { name: "Primary navigation" }).getByRole("link", { name: label }).click();
    await expect(page).toHaveURL(url);
    await expect(page.locator("main h1").first()).toBeVisible();
  }
  // Same document: the marker survives client-side navigation.
  expect(await page.evaluate(() => (window as unknown as { __navMarker: number }).__navMarker)).toBe(marker);
});

test("shows real runtime connection status", async ({ page }) => {
  await openApp(page, "/overview");
  await expect(page.getByRole("contentinfo")).toContainText("Runtime: Live");
  await expect(page.getByRole("contentinfo")).toContainText("Dispatching active");
});

test("command palette navigates and runs actions from the keyboard", async ({ page }) => {
  await openApp(page, "/overview");
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await expect(palette).toBeVisible();
  await page.keyboard.type("Worktrees");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/worktrees/);
  await expect(palette).toBeHidden();
});

test("switches theme and persists it across reloads", async ({ page }) => {
  await openApp(page, "/overview");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("button", { name: "Theme" }).click();
  await page.getByRole("menuitemradio", { name: "Light" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  const background = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(background).toBe("rgb(245, 246, 248)");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("switches language to Swedish from the command palette", async ({ page }) => {
  await openApp(page, "/overview");
  await page.keyboard.press("Control+k");
  await page.keyboard.type("svenska");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("navigation", { name: "Huvudnavigering" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Översikt" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "sv");
});

test("keyboard shortcuts toggle shell panels", async ({ page }) => {
  await openApp(page, "/overview");
  const nav = page.getByRole("navigation", { name: "Primary navigation" });
  const before = (await nav.boundingBox())!.width;
  await page.keyboard.press("Control+b");
  await expect.poll(async () => (await nav.boundingBox())!.width).toBeLessThan(before);
  await page.keyboard.press("Control+b");
  await expect(page.getByRole("region", { name: "Activity" })).toBeVisible();
});
