import { expect, test } from "@playwright/test";
import { api, openApp, prepareRuntime } from "./helpers.js";

test.beforeAll(async () => {
  await prepareRuntime();
});

// Runs against the real runtime and vault. The service is not reachable with this made-up key (offline
// it cannot be reached at all; online it refuses the key), so the connection ends as an error either
// way; what is checked is the path: key stored, profile saved, test run, result shown, nothing leaked.
test("a model connection is added from just an API key, tested, and removed again", async ({ page }) => {
  const key = "sk-or-e2e-0123456789abcdefghij";
  await openApp(page, "/settings");
  await page.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Model connections" }).click();
  await expect(page.getByRole("heading", { name: "Model connections" })).toBeVisible();
  await page.getByRole("button", { name: "Add connection" }).click();

  const dialog = page.getByRole("dialog");
  await dialog.getByLabel(/API key/).fill(key);
  await dialog.getByRole("button", { name: "Connect" }).click();
  await expect(dialog).toBeHidden({ timeout: 30_000 });

  const card = page.getByRole("region", { name: "OpenRouter" });
  await expect(card).toBeVisible();
  await expect(card.getByText("Error", { exact: true })).toBeVisible();
  await expect(card.getByText("Configured", { exact: true })).toBeVisible();
  // The key is in the vault (names only come back) and nowhere on the page.
  const credentials = await api<{ configured: string[] }>("GET", "/credentials");
  expect(credentials.configured).toContain("OPENROUTER_API_KEY");
  expect(await page.content()).not.toContain(key);
  const profiles = await api<{ status: string; credential_ref: string }[]>("GET", "/connection-profiles");
  expect(profiles).toHaveLength(1);
  expect(profiles[0]).toMatchObject({ status: "error", credential_ref: "OPENROUTER_API_KEY" });
  expect(JSON.stringify(profiles)).not.toContain(key);

  await card.getByRole("button", { name: "Remove OpenRouter" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Remove" }).click();
  await expect(page.getByText("No model connections yet")).toBeVisible();
  await api("DELETE", "/credentials/OPENROUTER_API_KEY");
});
