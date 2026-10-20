/**
 * JSON values carried in free-form API fields (event payloads, tracker data, approval policies).
 * The OpenAPI document types those fields as `unknown`; narrow them with these helpers.
 */
export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]?: JsonValue } | null;

/** The value as a JSON object, or `null` when it is not one. */
export function asJsonObject(value: unknown): { [key: string]: JsonValue | undefined } | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as { [key: string]: JsonValue | undefined })
    : null;
}
