import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { RemoteAccessStatus } from "@symphony/shared-types";
import { settings } from "../../test/fixtures.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { settingsMessages } from "./messages.js";
import { RemoteSection } from "./sections.js";
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
    fireEvent.click(within(dialog).getByRole("button", { name: m.revoke }));
    await waitFor(() => expect(runtime.calls.some((c) => c.method === "DELETE" && c.path === "/remote-access/token")).toBe(true));
    expect(await screen.findByText(m.states.token_missing!)).toBeInTheDocument();
  });
});
