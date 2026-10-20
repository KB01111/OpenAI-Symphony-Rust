import { describe, expect, it } from "vitest";
import { settings } from "../../test/fixtures.js";
import { settingsMessages } from "./messages.js";
import { isLoopback, portOf, serverErrorFields, validateSettings } from "./validation.js";

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
