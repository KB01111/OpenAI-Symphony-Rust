import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ConnectionProfileInput, ProviderProfile } from "@symphony/shared-types";
import { ConnectionsSection } from "./ConnectionsSection.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";

const KEY = "sk-or-ui-test-0123456789abcdef";
const ALLOWED = ["GITHUB_TOKEN", "OPENAI_API_KEY", "OPENROUTER_API_KEY", "OPENCODE_GO_API_KEY", "NVIDIA_API_KEY"];

function profile(overrides: Partial<ProviderProfile> = {}): ProviderProfile {
  return {
    id: "p1",
    name: "OpenRouter",
    provider: "opencode",
    model: null,
    command_override: null,
    approval_policy: "never",
    sandbox_mode: "workspace-write",
    credential_ref: "OPENROUTER_API_KEY",
    is_default: true,
    service: "openrouter",
    base_url: null,
    status: "configured",
    status_detail: null,
    verified_at: null,
    fallback_profile_id: null,
    ...overrides
  };
}

/** A runtime that keeps profiles and the configured keys in memory, like the real one. */
function runtime(initial: ProviderProfile[] = [], configured: string[] = [], testStatus: ProviderProfile["status"] = "verified") {
  let profiles = [...initial];
  let keys = [...configured];
  const models = ["anthropic/claude-sonnet-5.5", "openai/gpt-5.5"];
  const fromInput = (id: string, input: ConnectionProfileInput): ProviderProfile =>
    profile({
      id,
      name: input.name,
      service: input.service,
      base_url: input.base_url ?? null,
      model: input.model ?? null,
      fallback_profile_id: input.fallback_profile_id ?? null,
      is_default: input.is_default ?? false,
      credential_ref: "OPENROUTER_API_KEY"
    });
  return createMockRuntime({
    "GET /connection-profiles": () => profiles,
    "POST /connection-profiles": ({ body }) => {
      const created = fromInput(`p${profiles.length + 1}`, body as ConnectionProfileInput);
      profiles = [...profiles, created];
      return created;
    },
    "PUT /connection-profiles/:id": ({ path, body }) => {
      const id = path.split("/").pop()!;
      const updated = fromInput(id, body as ConnectionProfileInput);
      profiles = profiles.map((p) => (p.id === id ? { ...updated, status: p.status, status_detail: p.status_detail } : p));
      return profiles.find((p) => p.id === id);
    },
    "DELETE /connection-profiles/:id": () => {
      profiles = [];
      return undefined; // 204: the mock answers an undefined result with no content
    },
    "POST /connection-profiles/:id/test": () => {
      profiles = profiles.map((p) => ({ ...p, status: testStatus, status_detail: `${models.length} models` }));
      return { profile: profiles[0], models };
    },
    "GET /connection-profiles/:id/models": () => ({ status: "verified", detail: "2 models", models }),
    "GET /credentials": () => ({ configured: keys, allowed: ALLOWED }),
    "POST /credentials": ({ body }) => {
      keys = [...keys, (body as { key: string }).key];
      return { saved: true, key: (body as { key: string }).key };
    }
  });
}

const render = (rt: ReturnType<typeof runtime>) =>
  renderWithProviders(<ConnectionsSection />, { runtime: rt, route: "/settings", path: "/settings" });

describe("Model connections", () => {
  it("connects a service from just its API key: stores the key, saves the profile and tests it", async () => {
    const rt = runtime();
    render(rt);
    fireEvent.click(await screen.findByRole("button", { name: "Add connection" }));
    const dialog = await screen.findByRole("dialog");
    const key = within(dialog).getByLabelText(/API key/) as HTMLInputElement;
    expect(key.type).toBe("password");
    fireEvent.change(key, { target: { value: KEY } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Connect" }));

    await waitFor(() => expect(rt.calls.some((c) => c.method === "POST" && c.path.endsWith("/connection-profiles/p1/test"))).toBe(true));
    const credential = rt.calls.find((c) => c.method === "POST" && c.path.endsWith("/credentials"));
    expect(credential?.body).toEqual({ key: "OPENROUTER_API_KEY", value: KEY });
    const created = rt.calls.find((c) => c.method === "POST" && c.path.endsWith("/connection-profiles"));
    expect(created?.body).toMatchObject({ name: "OpenRouter", service: "openrouter", base_url: null, is_default: true });
    // The key is not part of the profile and is not left in the page.
    expect(JSON.stringify(created?.body)).not.toContain(KEY);
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(document.body.innerHTML).not.toContain(KEY);
    expect(await screen.findByRole("region", { name: "OpenRouter" })).toBeInTheDocument();
  });

  it("shows the test result as text and lets the user pick a listed model", async () => {
    const rt = runtime([profile({ status: "verified", status_detail: "2 models" })], ["OPENROUTER_API_KEY"]);
    render(rt);
    const card = await screen.findByRole("region", { name: "OpenRouter" });
    expect(within(card).getByText("Verified")).toBeInTheDocument();
    expect(within(card).getByText("2 models")).toBeInTheDocument();
    expect(within(card).getByText("Configured")).toBeInTheDocument();

    const save = within(card).getByRole("button", { name: "Save model" });
    expect(save).toBeDisabled();
    const combo = within(card).getByRole("combobox", { name: /Default model/ });
    fireEvent.change(combo, { target: { value: "openai/gpt-5.5" } });
    expect(save).not.toBeDisabled();
    fireEvent.click(save);
    await waitFor(() => {
      const put = rt.calls.find((c) => c.method === "PUT" && c.path.endsWith("/connection-profiles/p1"));
      expect(put?.body).toMatchObject({ name: "OpenRouter", service: "openrouter", model: "openai/gpt-5.5" });
    });
  });

  it("will not test a connection that has no key, and refuses an unsafe base URL before sending anything", async () => {
    const rt = runtime([profile()], []);
    render(rt);
    const card = await screen.findByRole("region", { name: "OpenRouter" });
    expect(within(card).getByText("Not set")).toBeInTheDocument();
    expect(within(card).getByRole("button", { name: "Test connection" })).toBeDisabled();

    fireEvent.click(within(card).getByRole("button", { name: "Edit OpenRouter" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/API key/), { target: { value: KEY } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Advanced" }));
    fireEvent.change(within(dialog).getByLabelText("Base URL"), { target: { value: "http://openrouter.ai/api/v1" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByText("The base URL must use https.")).toBeInTheDocument();
    expect(rt.calls.some((c) => c.method === "POST" || c.method === "PUT")).toBe(false);

    fireEvent.change(within(dialog).getByLabelText("Base URL"), { target: { value: "https://evil.example.com/v1" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByText("The base URL must be on openrouter.ai.")).toBeInTheDocument();
    expect(rt.calls.some((c) => c.method === "POST" || c.method === "PUT")).toBe(false);
  });

  it("removes a connection after confirmation", async () => {
    const rt = runtime([profile()], ["OPENROUTER_API_KEY"]);
    render(rt);
    const card = await screen.findByRole("region", { name: "OpenRouter" });
    fireEvent.click(within(card).getByRole("button", { name: "Remove OpenRouter" }));
    const confirm = await screen.findByRole("alertdialog");
    fireEvent.click(within(confirm).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(rt.calls.some((c) => c.method === "DELETE" && c.path.endsWith("/connection-profiles/p1"))).toBe(true));
    expect(await screen.findByText("No model connections yet")).toBeInTheDocument();
  });

  it("never stores a key typed for one service under another", async () => {
    const rt = runtime();
    render(rt);
    fireEvent.click(await screen.findByRole("button", { name: "Add connection" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/API key/), { target: { value: KEY } });
    // Switch to NVIDIA Build before connecting: the typed key is dropped, so Connect needs a new one.
    fireEvent.click(within(dialog).getByRole("combobox", { name: /Service/ }));
    fireEvent.click(await screen.findByRole("option", { name: "NVIDIA Build" }));
    expect((within(dialog).getByLabelText(/API key/) as HTMLInputElement).value).toBe("");
    fireEvent.click(within(dialog).getByRole("button", { name: "Connect" }));
    expect(await within(dialog).findByText("Enter the API key to connect.")).toBeInTheDocument();
    expect(rt.calls.some((c) => c.method === "POST")).toBe(false);
  });

  it("explains a pasted key with a control character instead of dropping it", async () => {
    const rt = runtime();
    render(rt);
    fireEvent.click(await screen.findByRole("button", { name: "Add connection" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/API key/), { target: { value: `${KEY}\0extra` } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Connect" }));
    expect(await within(dialog).findByText("The key must be a single line. Paste it again.")).toBeInTheDocument();
    expect(rt.calls.some((c) => c.method === "POST")).toBe(false);
  });

  it("retries after a failed test on the profile it already saved instead of creating another", async () => {
    const rt = runtime();
    let failures = 1;
    rt.on("POST /connection-profiles/:id/test", () => {
      if (failures-- > 0) return { status: 500, body: { error: { code: "boom", message: "boom" } } };
      return { profile: profile({ id: "p1", status: "verified" }), models: [] };
    });
    render(rt);
    fireEvent.click(await screen.findByRole("button", { name: "Add connection" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/API key/), { target: { value: KEY } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Connect" }));
    await waitFor(() => expect(rt.calls.filter((c) => c.path.endsWith("/test"))).toHaveLength(1));
    // The dialog stays open; connecting again updates p1 rather than creating p2.
    fireEvent.click(await within(dialog).findByRole("button", { name: "Connect" }));
    await waitFor(() => expect(rt.calls.filter((c) => c.path.endsWith("/test"))).toHaveLength(2));
    expect(rt.calls.filter((c) => c.method === "POST" && c.path.endsWith("/connection-profiles"))).toHaveLength(1);
    const put = rt.calls.find((c) => c.method === "PUT" && c.path.endsWith("/connection-profiles/p1"));
    expect(put).toBeDefined();
    // The retry keeps the name it was created with; it does not become "OpenRouter 2".
    expect(put?.body).toMatchObject({ name: "OpenRouter" });
  });

  it("keeps a stored key reachable after the last connection to its service is removed", async () => {
    const rt = runtime([profile()], ["OPENROUTER_API_KEY"]);
    render(rt);
    const card = await screen.findByRole("region", { name: "OpenRouter" });
    expect(screen.queryByText("Stored keys without a connection")).not.toBeInTheDocument();
    fireEvent.click(within(card).getByRole("button", { name: "Remove OpenRouter" }));
    const confirm = await screen.findByRole("alertdialog");
    fireEvent.click(within(confirm).getByRole("button", { name: "Remove" }));
    // The connection is gone, the key is not: it is offered for Replace/Remove.
    expect(await screen.findByText("Stored keys without a connection")).toBeInTheDocument();
    expect(screen.getByText("OPENROUTER_API_KEY")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove OPENROUTER_API_KEY" })).toBeInTheDocument();
    expect(rt.calls.some((c) => c.method === "DELETE" && c.path.endsWith("/credentials/OPENROUTER_API_KEY"))).toBe(false);
  });

  it("does not show connections as keyless when the key lookup failed", async () => {
    const rt = runtime([profile()], ["OPENROUTER_API_KEY"]);
    rt.on("GET /credentials", () => ({ status: 500, body: { error: { code: "boom", message: "vault unreadable" } } }));
    render(rt);
    expect(await screen.findByText("Could not check which API keys are stored")).toBeInTheDocument();
    expect(screen.queryByText("Not set")).not.toBeInTheDocument();
  });
});
