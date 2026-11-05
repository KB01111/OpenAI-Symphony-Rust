import { describe, expect, it } from "vitest";
import type { ProviderProfile } from "@symphony/shared-types";
import { SERVICES, baseUrlProblem, modelProblem, nextProfileName, serviceInfo, statusTone } from "./connectionLogic.js";

const openrouter = serviceInfo("openrouter")!;

describe("connection logic", () => {
  it("knows the three services and their vault keys", () => {
    expect(SERVICES.map((s) => s.credentialKey)).toEqual(["OPENROUTER_API_KEY", "OPENCODE_GO_API_KEY", "NVIDIA_API_KEY"]);
    expect(serviceInfo("nvidia_build")?.baseUrl).toBe("https://integrate.api.nvidia.com/v1");
  });

  it("accepts only https on the service's own host", () => {
    expect(baseUrlProblem(openrouter, "")).toBeNull();
    expect(baseUrlProblem(openrouter, "https://openrouter.ai/api/v1/")).toBeNull();
    expect(baseUrlProblem(openrouter, "http://openrouter.ai/api/v1")).toBe("insecure");
    expect(baseUrlProblem(openrouter, "https://openrouter.ai.evil.com/v1")).toBe("host");
    expect(baseUrlProblem(openrouter, "https://integrate.api.nvidia.com/v1")).toBe("host");
    expect(baseUrlProblem(openrouter, "https://user:pw@openrouter.ai/v1")).toBe("invalid");
    expect(baseUrlProblem(openrouter, "https://openrouter.ai:8443/v1")).toBe("invalid");
    expect(baseUrlProblem(openrouter, "not a url")).toBe("invalid");
    expect(baseUrlProblem(openrouter, "https://openrouter.ai/api/v1?")).toBe("invalid");
    expect(baseUrlProblem(openrouter, "https://openrouter.ai/api/v1#")).toBe("invalid");
  });

  it("names a second profile of a service differently", () => {
    const taken = [{ name: "OpenRouter" }, { name: "OpenRouter 2" }] as ProviderProfile[];
    expect(nextProfileName(openrouter, [])).toBe("OpenRouter");
    expect(nextProfileName(openrouter, taken)).toBe("OpenRouter 3");
  });

  it("checks model ids and maps statuses to tones", () => {
    expect(modelProblem("anthropic/claude-sonnet-5.5")).toBe(false);
    expect(modelProblem("")).toBe(false);
    expect(modelProblem("a model; rm")).toBe(true);
    expect(statusTone("verified")).toBe("success");
    expect(statusTone("limited")).toBe("warning");
    expect(statusTone("error")).toBe("danger");
    expect(statusTone("configured")).toBe("neutral");
  });
});
