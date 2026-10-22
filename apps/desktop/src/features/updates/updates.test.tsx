import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { useUiStore } from "../../store/uiStore.js";
import { ApplicationSection } from "../settings/sections.js";
import { settingsMessages } from "../settings/messages.js";
import { UpdateBanner } from "./UpdateBanner.js";
import { downloadPercent } from "./install.js";
import { updateMessages } from "./messages.js";

const m = settingsMessages.en.application;
const u = updateMessages.en;

// The host pushes progress on a Tauri event; capture the listener the app registers.
let progressListener: ((event: { payload: unknown }) => void) | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, handler: (event: { payload: unknown }) => void) => {
    progressListener = handler;
    return () => {
      progressListener = null;
    };
  })
}));

const update = (over: Record<string, unknown> = {}) => ({
  current: "2.0.0",
  latest: "2.1.0",
  update_available: true,
  prerelease: false,
  published_at: "2026-10-22T10:00:00Z",
  notes: "Faster startup",
  installable: true,
  ...over
});

const runtimeInfo = () => ({ service: "symphony", version: "2.0.0", api_version: 1, started_at: "2026-10-22T10:00:00Z", auth_required: true });

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

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

/** Lets a resolved or rejected check reach the UI before asserting that nothing was shown. */
const settle = () => act(async () => void (await new Promise((resolve) => setTimeout(resolve, 50))));

const calls = (invoke: ReturnType<typeof installHost>, command: string) => invoke.mock.calls.filter(([c]) => c === command);

beforeEach(() => {
  progressListener = null;
  useUiStore.setState({ autoCheckUpdates: true, dismissedUpdate: null, updateInstall: { phase: "idle" } });
});

afterEach(() => {
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("downloadPercent", () => {
  it("is unknown without a size and never exceeds 100", () => {
    expect(downloadPercent(50, null)).toBeNull();
    expect(downloadPercent(50, 0)).toBeNull();
    expect(downloadPercent(50, 200)).toBe(25);
    expect(downloadPercent(300, 200)).toBe(100);
  });
});

describe("UpdateBanner", () => {
  it("checks on startup, announces the new version and installs it in-app with progress", async () => {
    let finish: (v: unknown) => void = () => undefined;
    const invoke = installHost({
      check_for_updates: async () => update(),
      install_update: () => new Promise((resolve) => (finish = resolve))
    });
    renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });

    expect(await screen.findByText("Version 2.1.0 is available (you are running 2.0.0).")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: u.install }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("install_update", { confirmed: false }));

    await waitFor(() => expect(progressListener).not.toBeNull());
    act(() => progressListener?.({ payload: { phase: "downloading", downloaded: 50, total: 200 } }));
    expect(await screen.findByText("Downloading update… 25%")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: u.progressLabel })).toBeInTheDocument();
    // No second install can be started while one runs.
    expect(screen.queryByRole("button", { name: u.install })).not.toBeInTheDocument();

    act(() => progressListener?.({ payload: { phase: "installing", downloaded: 200, total: 200 } }));
    expect(await screen.findByText(u.installing)).toBeInTheDocument();
    await act(async () => finish({ status: "installed" }));
    expect(useUiStore.getState().updateInstall.phase).toBe("installing");
  });

  it("asks before interrupting running agents and installs only after confirmation", async () => {
    const invoke = installHost({
      check_for_updates: async () => update(),
      install_update: async (_cmd, args) => (args?.confirmed ? { status: "installed" } : { status: "needs_confirmation", running: 2 })
    });
    renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    fireEvent.click(await screen.findByRole("button", { name: u.install }));
    expect(await screen.findByText(u.confirmTitle)).toBeInTheDocument();
    expect(screen.getByText(/2 agent run\(s\) are in progress/)).toBeInTheDocument();
    expect(calls(invoke, "install_update")).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: u.confirmInstall }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("install_update", { confirmed: true }));
    await waitFor(() => expect(useUiStore.getState().updateInstall.phase).toBe("installing"));
  });

  it("cancelling the confirmation installs nothing", async () => {
    const invoke = installHost({
      check_for_updates: async () => update(),
      install_update: async () => ({ status: "needs_confirmation", running: 1 })
    });
    renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    fireEvent.click(await screen.findByRole("button", { name: u.install }));
    await screen.findByText(u.confirmTitle);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(useUiStore.getState().updateInstall.phase).toBe("idle"));
    expect(calls(invoke, "install_update")).toHaveLength(1);
  });

  it("shows a failed install with the reason and lets the user retry", async () => {
    let attempt = 0;
    const invoke = installHost({
      check_for_updates: async () => update(),
      install_update: async () => {
        attempt += 1;
        if (attempt === 1) throw "Download failed: the signature does not match";
        return { status: "installed" };
      }
    });
    renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    fireEvent.click(await screen.findByRole("button", { name: u.install }));
    expect(await screen.findByText(u.installFailed)).toBeInTheDocument();
    expect(screen.getByText(/signature does not match/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: u.retry }));
    await waitFor(() => expect(calls(invoke, "install_update")).toHaveLength(2));
    await waitFor(() => expect(useUiStore.getState().updateInstall.phase).toBe("installing"));
  });

  it("points to the release page when the release cannot be installed in-app", async () => {
    const invoke = installHost({
      check_for_updates: async () => update({ installable: false }),
      open_releases_page: async () => undefined
    });
    renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    expect(screen.queryByRole("button", { name: u.install })).not.toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: u.openReleases }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("open_releases_page", undefined));
    expect(calls(invoke, "install_update")).toHaveLength(0);
  });

  it("can be dismissed for one version and returns for a newer one", async () => {
    let latest = "2.1.0";
    installHost({ check_for_updates: async () => update({ latest }) });
    const view = renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    await screen.findByText(/Version 2.1.0 is available/);
    fireEvent.click(screen.getByRole("button", { name: u.later }));
    await waitFor(() => expect(screen.queryByText(/is available/)).not.toBeInTheDocument());
    expect(useUiStore.getState().dismissedUpdate).toBe("2.1.0");

    // A later release is announced again.
    latest = "2.2.0";
    view.unmount();
    renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    expect(await screen.findByText(/Version 2.2.0 is available/)).toBeInTheDocument();
  });

  it("stays silent when there is nothing newer, when the check fails, and outside the desktop host", async () => {
    // Nothing newer.
    let invoke = installHost({ check_for_updates: async () => update({ update_available: false, latest: "2.0.0" }) });
    let view = renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    await waitFor(() => expect(calls(invoke, "check_for_updates")).toHaveLength(1));
    await settle();
    expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
    view.unmount();

    // Offline: a failed automatic check never shows an error banner.
    invoke = installHost({
      check_for_updates: async () => {
        throw "Could not reach GitHub";
      }
    });
    view = renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    await waitFor(() => expect(calls(invoke, "check_for_updates")).toHaveLength(1));
    await settle();
    expect(screen.queryByText(/Could not reach GitHub/)).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    view.unmount();

    // Browser / dev server: no host, so nothing to ask.
    delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    await settle();
    expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
  });

  it("makes no request at all when automatic checks are turned off", async () => {
    useUiStore.setState({ autoCheckUpdates: false });
    const invoke = installHost({ check_for_updates: async () => update() });
    renderWithProviders(<UpdateBanner />, { runtime: createMockRuntime({}) });
    await settle();
    expect(calls(invoke, "check_for_updates")).toHaveLength(0);
  });
});

describe("Settings → Application update card", () => {
  it("installs from Settings, shares the check with the banner and toggles the automatic check", async () => {
    const invoke = installHost({
      check_for_updates: async () => update(),
      install_update: async () => ({ status: "installed" })
    });
    renderWithProviders(<ApplicationSection />, { runtime: createMockRuntime({ "GET /runtime": runtimeInfo }) });
    // Opening Settings alone is not a network request.
    await screen.findByRole("button", { name: m.checkUpdates });
    expect(calls(invoke, "check_for_updates")).toHaveLength(0);

    fireEvent.click(screen.getByRole("button", { name: m.checkUpdates }));
    expect(await screen.findByText("Version 2.1.0 is available. You are running 2.0.0.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: u.install }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("install_update", { confirmed: false }));

    const toggle = screen.getByRole("switch", { name: m.autoCheckUpdates });
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    expect(useUiStore.getState().autoCheckUpdates).toBe(false);
  });
});
