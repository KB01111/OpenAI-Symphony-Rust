import { describe, expect, it } from "vitest";
import { formatMessage } from "@symphony/shared-types";

describe("formatMessage", () => {
  it("fills placeholders and leaves unknown ones untouched", () => {
    expect(formatMessage("{a} and {b}", { a: 1 })).toBe("1 and {b}");
  });

  it("chooses the plural form by count, with exact matches first", () => {
    const t = "{count, plural, =0 {no tasks} one {# task} other {# tasks}} in {status}";
    expect(formatMessage(t, { count: 0, status: "Ready" })).toBe("no tasks in Ready");
    expect(formatMessage(t, { count: 1, status: "Ready" })).toBe("1 task in Ready");
    expect(formatMessage(t, { count: 12, status: "Ready" })).toBe("12 tasks in Ready");
    expect(formatMessage("{count, plural, one {# körning} other {# körningar}}", { count: 2 })).toBe("2 körningar");
  });

  it("keeps a plural block whose count is missing", () => {
    const t = "{count, plural, one {# run} other {# runs}}";
    expect(formatMessage(t, {})).toBe(t);
  });
});
