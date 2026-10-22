import { expect, test } from "@playwright/test";
import { openApp, prepareRuntime } from "./helpers.js";

test.beforeEach(async () => {
  await prepareRuntime();
});

test("keyboard shortcut help opens with F1, from the palette and from Settings, and closes with Escape", async ({ page }) => {
  await openApp(page, "/overview");
  await page.keyboard.press("F1");
  const dialog = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Command palette and search")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  await page.keyboard.press("Control+K");
  await page.getByRole("option", { name: /Keyboard shortcuts/ }).click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");

  await openApp(page, "/settings");
  await page.getByRole("button", { name: "Show keyboard shortcuts" }).click();
  await expect(dialog).toBeVisible();
});
