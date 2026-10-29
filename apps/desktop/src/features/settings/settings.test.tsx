import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { RemoteAccessStatus } from "@symphony/shared-types";
import { settings } from "../../test/fixtures.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { settingsMessages } from "./messages.js";
import { BackupsSection, RemoteSection } from "./sections.js";
import SettingsPage from "./SettingsPage.js";
import { isLoopback, isValidRemoteOrigin, parseOrigins, portOf, serverErrorFields, validateSettings } from "./validation.js";

const v = settingsMessages.en.validation;

describe("settings validation (mirror of the runtime's validate_settings)", () => {
  it("accepts the defaults", () => {
    expect(validateSettings(settings(), v)).toEqual({});
  });

  it("flags out-of-range values per field", () => {
    const s = settings();
    const errors = validateSettings(
      {
        ...s,
        polling_interval_ms: 500,
        remote_port: 70000,
        remote_bind_address: " ",
        resource_policy: { ...s.resource_policy, max_concurrent_on_ac: 2, max_concurrent_on_battery: 5, high_cpu_threshold_percent: 5 }
      },
      v
    );
    expect(Object.keys(errors).sort()).toEqual(
      ["high_cpu_threshold_percent", "max_concurrent_on_battery", "polling_interval_ms", "remote_bind_address", "remote_port"].sort()
    );
  });

  it("accepts 0 (keep everything) and up to ten years of activity history, and rejects the rest", () => {
    for (const days of [0, 1, 30, 3650]) expect(validateSettings(settings({ event_retention_days: days }), v)).toEqual({});
    // A runtime that predates the setting omits it: that is the default, not an error.
    expect(validateSettings(settings({ event_retention_days: undefined }), v)).toEqual({});
    for (const days of [-1, 3651]) expect(validateSettings(settings({ event_retention_days: days }), v)).toHaveProperty("event_retention_days", v.retention);
    expect(validateSettings(settings({ event_retention_days: 1.5 }), v)).toHaveProperty("event_retention_days", v.integer);
    expect(serverErrorFields("event_retention_days must be 0 (keep everything) or at most 3650")).toEqual(["event_retention_days"]);
  });

  it("bounds the backup schedule and the pull request refresh like the runtime", () => {
    expect(validateSettings(settings({ backup_interval_hours: undefined, backup_keep: undefined, pr_refresh_interval_minutes: undefined }), v)).toEqual({});
    for (const hours of [0, 1, 720]) expect(validateSettings(settings({ backup_interval_hours: hours }), v)).toEqual({});
    expect(validateSettings(settings({ backup_interval_hours: 721 }), v)).toHaveProperty("backup_interval_hours", v.backupInterval);
    for (const keep of [0, 101]) expect(validateSettings(settings({ backup_keep: keep }), v)).toHaveProperty("backup_keep", v.backupKeep);
    for (const minutes of [0, 5, 1440]) expect(validateSettings(settings({ pr_refresh_interval_minutes: minutes }), v)).toEqual({});
    for (const minutes of [1, 4, 1441]) {
      expect(validateSettings(settings({ pr_refresh_interval_minutes: minutes }), v)).toHaveProperty("pr_refresh_interval_minutes", v.prRefresh);
    }
    expect(serverErrorFields("pr_refresh_interval_minutes must be 0 (only when asked) or between 5 and 1440")).toEqual(["pr_refresh_interval_minutes"]);
    expect(serverErrorFields("backup_keep must be between 1 and 100")).toEqual(["backup_keep"]);
  });

  it("rejects a remote port equal to the local API port", () => {
    expect(validateSettings(settings({ remote_port: 4780 }), v, 4780)).toHaveProperty("remote_port");
    expect(validateSettings(settings({ remote_port: 47821 }), v, 4780)).toEqual({});
    expect(portOf("http://127.0.0.1:4780")).toBe(4780);
  });

  it("maps runtime errors to fields and recognises loopback binds", () => {
    expect(serverErrorFields("invalid_settings: remote_port must be between 1 and 65535")).toEqual(["remote_port"]);
    expect(isLoopback("127.0.0.1")).toBe(true);
    expect(isLoopback("0.0.0.0")).toBe(false);
  });
});

describe("remote access validation (mirror of validate_remote_settings)", () => {
  it("requires a loopback bind address", () => {
    expect(validateSettings(settings({ remote_bind_address: "0.0.0.0" }), v)).toHaveProperty("remote_bind_address", v.bindLoopback);
    expect(validateSettings(settings({ remote_bind_address: "192.168.1.20" }), v)).toHaveProperty("remote_bind_address");
    expect(validateSettings(settings({ remote_bind_address: "::1" }), v)).toEqual({});
    expect(isLoopback("127.0.0.1.evil.example")).toBe(false);
    expect(isLoopback("127.999.999.999")).toBe(false);
  });

  it("accepts only bare https origins (http for localhost) and never wildcards", () => {
    for (const ok of ["https://companion.example.com", "https://c.example.com:8443", "http://localhost:1421", "tauri://localhost"]) {
      expect(isValidRemoteOrigin(ok)).toBe(true);
    }
    for (const bad of ["*", "https://*.example.com", "http://192.168.1.5", "https://example.com/", "https://example.com/app", "null"]) {
      expect(isValidRemoteOrigin(bad)).toBe(false);
    }
    expect(validateSettings(settings({ remote_allowed_origins: ["*"] }), v)).toHaveProperty("remote_allowed_origins");
    expect(parseOrigins(" https://a.example.com\n\nhttps://b.example.com, ")).toEqual(["https://a.example.com", "https://b.example.com"]);
    expect(serverErrorFields("invalid_settings: '*': wildcard origins are not allowed")).toEqual(["remote_allowed_origins"]);
  });
});

function remoteStatus(overrides: Partial<RemoteAccessStatus> = {}): RemoteAccessStatus {
  return {
    enabled: true,
    bind_address: "127.0.0.1",
    port: 47821,
    allowed_origins: [],
    token_configured: false,
    state: "token_missing",
    listening_on: null,
    error: null,
    ...overrides
  };
}

describe("RemoteSection", () => {
  const m = settingsMessages.en.remote;

  it("shows the draft's origins again after a reset", async () => {
    const runtime = createMockRuntime({ "GET /remote-access": () => remoteStatus() });
    const Harness = () => {
      const [draft, setDraft] = useState(settings({ remote_allowed_origins: ["https://old.example.com"] }));
      return (
        <>
          <button type="button" onClick={() => setDraft(settings({ remote_allowed_origins: [] }))}>
            reset-draft
          </button>
          <RemoteSection draft={draft} errors={{}} update={(fn) => setDraft(fn)} />
        </>
      );
    };
    renderWithProviders(<Harness />, { runtime });
    expect(await screen.findByDisplayValue("https://old.example.com")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "reset-draft" }));
    await waitFor(() => expect(screen.queryByDisplayValue("https://old.example.com")).not.toBeInTheDocument());
  });

  it("issues a token once, then shows the running listener without echoing the token", async () => {
    let status = remoteStatus();
    const runtime = createMockRuntime({
      "GET /remote-access": () => status,
      "POST /remote-access/token": () => {
        status = remoteStatus({ token_configured: true, state: "running", listening_on: "127.0.0.1:47821" });
        return { token: "f".repeat(64), status };
      }
    });
    renderWithProviders(<RemoteSection draft={settings({ remote_api_enabled: true })} errors={{}} update={() => {}} />, { runtime });

    expect(await screen.findByText(m.states.token_missing!)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: m.generate }));
    expect(await screen.findByDisplayValue("f".repeat(64))).toBeInTheDocument();
    expect(screen.getByText(m.states.running!)).toBeInTheDocument();

    // Dismissing removes the only copy from the page.
    fireEvent.click(screen.getByRole("button", { name: m.done }));
    expect(screen.queryByDisplayValue("f".repeat(64))).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: m.rotate })).toBeInTheDocument();
  });

  it("asks before revoking and reports a failed listener", async () => {
    let status = remoteStatus({ token_configured: true, state: "failed", error: "Could not listen on 127.0.0.1:47821: address in use" });
    const runtime = createMockRuntime({
      "GET /remote-access": () => status,
      "DELETE /remote-access/token": () => (status = remoteStatus({ state: "token_missing" }))
    });
    renderWithProviders(<RemoteSection draft={settings({ remote_api_enabled: true })} errors={{}} update={() => {}} />, { runtime });

    expect(await screen.findByText(/address in use/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: m.revoke }));
    const dialog = await screen.findByRole("alertdialog");
    expect(runtime.calls.some((c) => c.method === "DELETE")).toBe(false);
    fireEvent.click(await within(dialog).findByRole("button", { name: m.revoke }));
    await waitFor(() => expect(runtime.calls.some((c) => c.method === "DELETE" && c.path === "/remote-access/token")).toBe(true));
    expect(await screen.findByText(m.states.token_missing!)).toBeInTheDocument();
  });
});

describe("BackupsSection", () => {
  const m = settingsMessages.en.backups;
  const summary = (id: string, createdAt: string, scheduled = false) => ({
    id,
    path: `C:\\data\\backups\\${id}`,
    created_at: createdAt,
    app_version: "2.0.0",
    schema_version: 2,
    size_bytes: 2048,
    includes_workflow: true,
    scheduled
  });

  /** The section with a live draft, as the settings page renders it. */
  const Backups = ({ onDraft }: { onDraft?: (draft: ReturnType<typeof settings>) => void }) => {
    const [draft, setDraft] = useState(settings());
    return (
      <BackupsSection
        draft={draft}
        errors={{}}
        update={(fn) =>
          setDraft((current) => {
            const next = fn(current);
            onDraft?.(next);
            return next;
          })
        }
      />
    );
  };

  it("edits the automatic backup schedule and marks automatic backups", async () => {
    const runtime = createMockRuntime({
      "GET /backups": () => ({
        directory: "C:\\data\\backups",
        backups: [summary("symphony-backup-20261022T030000Z", "2026-10-22T03:00:00Z", true)]
      })
    });
    let latest: ReturnType<typeof settings> | null = null;
    renderWithProviders(<Backups onDraft={(d) => (latest = d)} />, { runtime });
    expect(await screen.findByText(m.scheduled)).toBeInTheDocument();
    const interval = screen.getByRole("spinbutton", { name: m.interval });
    expect(interval).toHaveValue(24);
    fireEvent.change(interval, { target: { value: "6" } });
    fireEvent.blur(interval);
    await waitFor(() => expect(latest?.backup_interval_hours).toBe(6));
    expect(screen.getByRole("spinbutton", { name: m.keep })).toHaveValue(7);
  });

  it("creates a backup, lists it and states what is not included", async () => {
    const stored = [summary("symphony-backup-20261020T100000Z", "2026-10-20T10:00:00Z")];
    const runtime = createMockRuntime({
      "GET /backups": () => ({ directory: "C:\\data\\backups", backups: stored }),
      "POST /backups": () => {
        const created = summary("symphony-backup-20261021T120000Z", "2026-10-21T12:00:00Z");
        stored.unshift(created);
        return { status: 201, body: created };
      }
    });
    renderWithProviders(<Backups />, { runtime });

    expect(await screen.findByText("symphony-backup-20261020T100000Z")).toBeInTheDocument();
    expect(screen.getByText(/API keys and tokens/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: m.create }));
    expect(await screen.findByText("symphony-backup-20261021T120000Z")).toBeInTheDocument();
    expect(runtime.calls.filter((c) => c.method === "POST")).toHaveLength(1);
  });

  it("reports a failed backup instead of pretending it succeeded", async () => {
    const runtime = createMockRuntime({
      "GET /backups": () => ({ directory: "C:\\data\\backups", backups: [] }),
      "POST /backups": () => ({
        status: 500,
        body: { error: { code: "storage_error", message: "storage_error: database backup failed: disk full" } }
      })
    });
    renderWithProviders(<Backups />, { runtime });
    expect(await screen.findByText(m.empty)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: m.create }));
    expect(await screen.findByText(m.createFailed)).toBeInTheDocument();
    expect(await screen.findByText(/disk full/)).toBeInTheDocument();
    expect(screen.getByText(m.empty)).toBeInTheDocument();
  });

  it("says so when the host does not manage backups", async () => {
    const runtime = createMockRuntime({
      "GET /backups": () => ({
        status: 501,
        body: { error: { code: "backups_unavailable", message: "backups_unavailable: This runtime host does not manage backups" } }
      })
    });
    renderWithProviders(<Backups />, { runtime });
    expect(await screen.findByText(m.unavailable)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: m.create })).not.toBeInTheDocument();
  });
});

describe("SettingsPage", () => {
  const renderPage = () => {
    const rt = createMockRuntime({ "GET /settings": () => settings(), "GET /providers": () => [] });
    renderWithProviders(<SettingsPage />, { runtime: rt, route: "/settings" });
  };
  const openSection = (name: string) => fireEvent.click(within(screen.getByRole("navigation", { name: "Settings sections" })).getByRole("button", { name }));

  it("shows one section at a time", async () => {
    renderPage();
    expect(await screen.findByRole("heading", { name: "Appearance" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Backups" })).not.toBeInTheDocument();
    openSection("Backups");
    expect(await screen.findByRole("heading", { name: "Backups" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Appearance" })).not.toBeInTheDocument();
  });

  it("keeps every section reachable below the md width through a picker", async () => {
    renderPage();
    const picker = await screen.findByRole("combobox", { name: "Settings sections" });
    fireEvent.change(picker, { target: { value: "backups" } });
    expect(await screen.findByRole("heading", { name: "Backups" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Appearance" })).not.toBeInTheDocument();
  });

  it("keeps Remote access mounted while hidden, so an issued one-time token is not lost", async () => {
    renderPage();
    await screen.findByRole("heading", { name: "Appearance" });
    const hiddenHeading = screen.getByRole("heading", { name: "Remote access", hidden: true });
    expect(hiddenHeading.closest("[hidden]")).not.toBeNull();
    openSection("Remote access");
    expect(screen.getByRole("heading", { name: "Remote access" }).closest("[hidden]")).toBeNull();
  });

  it("offers Reset only once something has been edited", async () => {
    renderPage();
    await screen.findByRole("heading", { name: "Appearance" });
    openSection("Scheduling & resources");
    const polling = await screen.findByLabelText("Polling interval (ms)");
    expect(screen.queryByRole("button", { name: "Reset" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
    fireEvent.change(polling, { target: { value: "45000" } });
    expect(await screen.findByRole("button", { name: "Reset" })).toBeEnabled();
  });

  it("marks a section that has an invalid field even while another section is open", async () => {
    renderPage();
    await screen.findByRole("heading", { name: "Appearance" });
    openSection("Scheduling & resources");
    fireEvent.change(await screen.findByLabelText("Polling interval (ms)"), { target: { value: "10" } });
    openSection("Appearance");
    const nav = screen.getByRole("navigation", { name: "Settings sections" });
    expect(within(within(nav).getByRole("button", { name: /Scheduling & resources/ })).getByRole("img", { name: "Has a field that needs attention" })).toBeInTheDocument();
    expect(within(within(nav).getByRole("button", { name: /Appearance/ })).queryByRole("img")).not.toBeInTheDocument();
  });
});
