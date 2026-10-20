import "@testing-library/jest-dom/vitest";

if (typeof window !== "undefined") {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: (query: string) => ({
      matches: query.includes("dark"),
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false
    })
  });
}

if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

class MockEventSource {
  public onmessage: ((ev: MessageEvent) => void) | null = null;
  public onerror: ((ev: Event) => void) | null = null;
  public readyState = 1;
  constructor(public readonly url: string) {}
  close() {
    this.readyState = 2;
  }
}

Object.defineProperty(globalThis, "EventSource", {
  writable: true,
  value: MockEventSource
});

// Provide deterministic fast mock responses in unit tests when no local server is bound
const defaultMockFetch = async (input: RequestInfo | URL): Promise<Response> => {
  const url = String(input);
  let body: unknown = {};
  if (url.endsWith("/api/v1/state")) {
    body = {
      generated_at: new Date().toISOString(),
      scheduler_paused: false,
      counts: {
        running: 1,
        retrying: 0,
        queued: 1,
        blocked: 0,
        human_review: 1,
        completed: 3,
        failed: 0
      },
      running: [],
      retrying: [],
      codex_totals: {
        input_tokens: 1200,
        output_tokens: 450,
        total_tokens: 1650,
        estimated_cost_usd: 0.0085,
        seconds_running: 14.2
      },
      rate_limits: null,
      hardware: {
        on_ac_power: true,
        battery_percent: 95,
        cpu_usage_percent: 14.5,
        memory_used_mb: 11200,
        memory_total_mb: 32768,
        available_memory_mb: 21568,
        memory_usage_percent: 34.1,
        thermal_throttling: false,
        recommended_max_concurrency: 2,
        sampled_at: new Date().toISOString()
      },
      recent_events: []
    };
  } else if (
    url.includes("/api/v1/projects") ||
    url.includes("/api/v1/tasks") ||
    url.includes("/api/v1/approvals") ||
    url.includes("/api/v1/prs") ||
    url.includes("/api/v1/providers")
  ) {
    body = [];
  } else if (url.includes("/api/v1/settings")) {
    body = {
      theme: "dark",
      language: "en",
      close_action: "minimize_to_tray",
      start_on_login: false,
      default_provider: "codex",
      remote_access_enabled: false,
      remote_bind_address: "127.0.0.1:47821",
      notifications_enabled: true,
      resource_policy: {
        max_agents_ac: 2,
        max_agents_battery: 1,
        memory_pressure_threshold_percent: 85,
        min_free_memory_mb: 2048,
        reduce_on_thermal_throttle: true
      },
      onboarding_completed: true
    };
  } else if (url.includes("/api/v1/workflow")) {
    body = {
      path: "WORKFLOW.md",
      raw_content: "---\ntracker:\n  kind: manual\n---\nPrompt",
      valid: true,
      validation_error: null,
      rendered_prompt_preview: "Prompt",
      parsed_config: {}
    };
  }

  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" }
  });
};

globalThis.fetch = defaultMockFetch as typeof fetch;

