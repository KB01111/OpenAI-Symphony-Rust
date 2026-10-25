import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AppSettings } from "@symphony/shared-types";
import ProvidersPage from "./ProvidersPage.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { providerStatus, settings } from "../../test/fixtures.js";

function providers() {
  const base = providerStatus();
  return [
    providerStatus({
      provider: "codex",
      display_name: "OpenAI Codex",
      installed: true,
      binary_path: "C:\\tools\\codex.exe",
      version: "0.42.0",
      auth_state: "unknown",
      auth_detail: "OPENAI_API_KEY environment variable not set",
      default_model: "gpt-5-codex",
      capabilities: {
        ...base.capabilities,
        provider: "codex",
        supports_mid_turn_pause: false,
        supports_dynamic_tools: true,
        models: ["gpt-5-codex", "gpt-5"]
      }
    }),
    providerStatus({
      provider: "claude_code",
      display_name: "Claude Code",
      installed: false,
      binary_path: null,
      version: null,
      auth_state: "unauthenticated",
      auth_detail: null,
      default_model: null,
      capabilities: {
        ...base.capabilities,
        provider: "claude_code",
        available: false,
        supports_approvals: false,
        models: []
      }
    })
  ];
}

function runtime(overrides: Partial<AppSettings> = {}) {
  let current = settings(overrides);
  return createMockRuntime({
    "GET /providers": () => providers(),
    "GET /settings": () => current,
    "PUT /settings": ({ body }) => {
      current = body as AppSettings;
      return current;
    },
    "GET /credentials": () => ({ configured: ["ANTHROPIC_API_KEY"], allowed: ["GITHUB_TOKEN", "LINEAR_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"] }),
    "POST /credentials": ({ body }) => ({ saved: true, key: (body as { key: string }).key })
  });
}

describe("ProvidersPage", () => {
  it("renders installation, authentication and capabilities from runtime data", async () => {
    renderWithProviders(<ProvidersPage />, { runtime: runtime(), route: "/providers", path: "/providers" });
    const codex = await screen.findByRole("region", { name: "OpenAI Codex" });
    expect(within(codex).getByText("C:\\tools\\codex.exe")).toBeInTheDocument();
    expect(within(codex).getByText("0.42.0")).toBeInTheDocument();
    // `unknown` must read as unverified, never as authenticated.
    expect(within(codex).getByText("Unverified")).toBeInTheDocument();
    expect(within(codex).queryByText("Authenticated")).not.toBeInTheDocument();
    const caps = within(codex).getByRole("list", { name: "OpenAI Codex capabilities" });
    expect(within(caps).getByText("Mid-turn pause").parentElement).toHaveTextContent("Not supported");
    expect(within(caps).getByText("Dynamic tools").parentElement).toHaveTextContent("Supported");

    const claude = screen.getByRole("region", { name: "Claude Code" });
    expect(within(claude).getAllByText("Not found on PATH").length).toBeGreaterThan(0);
    expect(within(claude).getByText("npm install -g @anthropic-ai/claude-code")).toBeInTheDocument();
    expect(within(claude).getByText("Not signed in")).toBeInTheDocument();
    expect(within(claude).getByText("ANTHROPIC_API_KEY")).toBeInTheDocument();
    expect(within(claude).getByText("Configured")).toBeInTheDocument();
  });

  it("saves a provider default model into settings", async () => {
    const rt = runtime({ provider_default_models: { claude_code: "sonnet" } });
    renderWithProviders(<ProvidersPage />, { runtime: rt, route: "/providers", path: "/providers" });
    const codex = await screen.findByRole("region", { name: "OpenAI Codex" });
    const combo = within(codex).getByRole("combobox", { name: /Default model/ });
    await waitFor(() => expect(combo).not.toBeDisabled());
    fireEvent.change(combo, { target: { value: "gpt-5" } });
    fireEvent.click(within(codex).getByRole("button", { name: "Save default" }));
    await waitFor(() => expect(rt.calls.some((c) => c.method === "PUT" && c.path === "/settings")).toBe(true));
    const put = rt.calls.find((c) => c.method === "PUT" && c.path === "/settings")!;
    expect((put.body as AppSettings).provider_default_models).toEqual({ claude_code: "sonnet", codex: "gpt-5" });
    expect((put.body as AppSettings).polling_interval_ms).toBe(30000);
  });

  it("makes a provider the global default without borrowing another provider's model", async () => {
    const rt = runtime({ default_provider: "codex", default_model: "gpt-5-codex" });
    rt.on("GET /providers", () => providers().map((p) => (p.provider === "claude_code" ? { ...p, installed: true } : p)));
    renderWithProviders(<ProvidersPage />, { runtime: rt, route: "/providers", path: "/providers" });
    const claude = await screen.findByRole("region", { name: "Claude Code" });
    const button = within(claude).getByRole("button", { name: "Use as global default" });
    await waitFor(() => expect(button).not.toBeDisabled());
    fireEvent.click(button);
    await waitFor(() => expect(rt.calls.some((c) => c.method === "PUT" && c.path === "/settings")).toBe(true));
    const put = rt.calls.find((c) => c.method === "PUT" && c.path === "/settings")!;
    expect((put.body as AppSettings).default_provider).toBe("claude_code");
    // Claude Code's own default applies, not Codex's model.
    expect((put.body as AppSettings).default_model).toBe("");
  });

  it("stores an API key through the vault and clears the input", async () => {
    const rt = runtime();
    renderWithProviders(<ProvidersPage />, { runtime: rt, route: "/providers", path: "/providers" });
    const codex = await screen.findByRole("region", { name: "OpenAI Codex" });
    fireEvent.click(await within(codex).findByRole("button", { name: "Set key… OPENAI_API_KEY" }));
    const dialog = await screen.findByRole("dialog");
    const input = within(dialog).getByLabelText(/Secret value/) as HTMLInputElement;
    expect(input.type).toBe("password");
    fireEvent.change(input, { target: { value: "sk-test-secret" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Store in vault" }));
    expect(input.value).toBe("");
    await waitFor(() => expect(rt.calls.some((c) => c.method === "POST" && c.path === "/credentials")).toBe(true));
    const post = rt.calls.find((c) => c.method === "POST" && c.path === "/credentials")!;
    expect(post.body).toEqual({ key: "OPENAI_API_KEY", value: "sk-test-secret" });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(document.body.textContent).not.toContain("sk-test-secret");
  });
});
