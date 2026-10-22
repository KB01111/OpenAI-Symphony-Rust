import { fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { useUiStore } from "../../store/uiStore.js";
import { settingsMessages } from "./messages.js";
import { ApplicationSection } from "./sections.js";

const m = settingsMessages.en.application;

const runtimeInfo = () => ({
  service: "symphony",
  version: "2.0.0",
  api_version: 1,
  started_at: "2026-10-22T10:00:00Z",
  auth_required: true
});

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** Stands in for the Tauri bridge: only the commands the host registers are answered. */
function installHost(handlers: Record<string, Invoke>) {
  const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "get_local_api_config") {
      return { base_url: "http://127.0.0.1:4780", token: "t", api_version: 1, mode: "embedded", discovery_error: null };
    }
    const handler = handlers[cmd];
    if (!handler) throw new Error(`unexpected command ${cmd}`);
    return handler(cmd, args);
  });
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { invoke };
  return invoke;
}

beforeEach(() => {
  useUiStore.setState({ shortcutsOpen: false });
});

afterEach(() => {
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("ApplicationSection: native host actions", () => {
  it("offers no folder or update actions outside the desktop host", async () => {
    renderWithProviders(<ApplicationSection />, { runtime: createMockRuntime({ "GET /runtime": runtimeInfo }) });
    expect(await screen.findByText("2.0.0")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: m.checkUpdates })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: m.openData })).not.toBeInTheDocument();
    // Shortcut help works everywhere.
    fireEvent.click(screen.getByRole("button", { name: m.showShortcuts }));
    expect(useUiStore.getState().shortcutsOpen).toBe(true);
  });

  it("opens the data and log folders through argument-free host commands", async () => {
    const invoke = installHost({
      open_data_directory: async () => "C:\\Users\\a\\AppData\\Local\\Symphony",
      open_log_directory: async () => "C:\\Users\\a\\AppData\\Local\\Symphony\\logs"
    });
    renderWithProviders(<ApplicationSection />, { runtime: createMockRuntime({ "GET /runtime": runtimeInfo }) });
    fireEvent.click(await screen.findByRole("button", { name: m.openData }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("open_data_directory", undefined));
    fireEvent.click(screen.getByRole("button", { name: m.openLogs }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("open_log_directory", undefined));
  });

  it("reports an available update with its notes and opens only the fixed release page", async () => {
    const invoke = installHost({
      check_for_updates: async () => ({
        current: "2.0.0",
        latest: "2.1.0",
        update_available: true,
        prerelease: false,
        published_at: "2026-10-22T10:00:00Z",
        notes: "Faster startup"
      }),
      open_releases_page: async () => undefined
    });
    renderWithProviders(<ApplicationSection />, { runtime: createMockRuntime({ "GET /runtime": runtimeInfo }) });
    // The host is only contacted when asked.
    await screen.findByRole("button", { name: m.checkUpdates });
    expect(invoke).not.toHaveBeenCalledWith("check_for_updates", expect.anything());

    fireEvent.click(screen.getByRole("button", { name: m.checkUpdates }));
    expect(await screen.findByText("Version 2.1.0 is available. You are running 2.0.0.")).toBeInTheDocument();
    expect(screen.getByText("Faster startup")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: m.openReleases }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("open_releases_page", undefined));
  });

  it("names the release page, not a folder, when it cannot be opened", async () => {
    installHost({
      check_for_updates: async () => ({ current: "2.0.0", latest: "2.1.0", update_available: true, prerelease: false, published_at: null, notes: null }),
      open_releases_page: async () => {
        throw "Failed to open explorer: not found";
      }
    });
    renderWithProviders(<ApplicationSection />, { runtime: createMockRuntime({ "GET /runtime": runtimeInfo }) });
    fireEvent.click(await screen.findByRole("button", { name: m.checkUpdates }));
    fireEvent.click(await screen.findByRole("button", { name: m.openReleases }));
    expect(await screen.findByText(m.openReleasesFailed)).toBeInTheDocument();
    expect(screen.queryByText(m.openFailed)).not.toBeInTheDocument();
  });

  it("says when the installed version is current and when nothing is published", async () => {
    let answer: unknown = { current: "2.0.0", latest: "2.0.0", update_available: false, prerelease: false, published_at: null, notes: null };
    installHost({ check_for_updates: async () => answer });
    renderWithProviders(<ApplicationSection />, { runtime: createMockRuntime({ "GET /runtime": runtimeInfo }) });
    fireEvent.click(await screen.findByRole("button", { name: m.checkUpdates }));
    expect(await screen.findByText("You are up to date (version 2.0.0).")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: m.openReleases })).not.toBeInTheDocument();

    answer = { current: "2.0.0", latest: null, update_available: false, prerelease: false, published_at: null, notes: null };
    fireEvent.click(screen.getByRole("button", { name: m.checkUpdates }));
    expect(await screen.findByText(m.noReleases)).toBeInTheDocument();
  });

  it("shows a failed check instead of claiming the app is up to date", async () => {
    installHost({
      check_for_updates: async () => {
        throw "Could not reach GitHub: connection refused";
      }
    });
    renderWithProviders(<ApplicationSection />, { runtime: createMockRuntime({ "GET /runtime": runtimeInfo }) });
    fireEvent.click(await screen.findByRole("button", { name: m.checkUpdates }));
    expect(await screen.findByText(m.checkFailed)).toBeInTheDocument();
    expect(screen.getByText(/connection refused/)).toBeInTheDocument();
    expect(screen.queryByText(/up to date/)).not.toBeInTheDocument();
  });
});
