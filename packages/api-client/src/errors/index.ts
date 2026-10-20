import type { ApiErrorBody } from "@symphony/shared-types";

export type ApiErrorKind =
  | "unauthorized"
  | "not_found"
  | "conflict"
  | "validation"
  | "server"
  | "network"
  | "aborted";

/** Error raised for any failed runtime request. `code` is the Rust `SymphonyError::code()`. */
export class SymphonyApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly kind: ApiErrorKind;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "SymphonyApiError";
    this.status = status;
    this.code = code;
    this.kind = classify(status, code);
  }

  static fromBody(status: number, body: unknown): SymphonyApiError {
    const parsed = body as Partial<ApiErrorBody> | null;
    const code = parsed?.error?.code ?? `http_${status}`;
    const message = parsed?.error?.message ?? `HTTP ${status}`;
    return new SymphonyApiError(status, code, message);
  }

  static network(cause: unknown): SymphonyApiError {
    const err = new SymphonyApiError(
      0,
      "network_error",
      cause instanceof Error ? cause.message : "Network request failed"
    );
    return err;
  }
}

function classify(status: number, code: string): ApiErrorKind {
  if (code === "aborted") return "aborted";
  if (status === 0) return "network";
  if (status === 401 || status === 403) return "unauthorized";
  if (status === 404) return "not_found";
  if (status === 409) return "conflict";
  if (status >= 400 && status < 500) return "validation";
  return "server";
}

export function isSymphonyApiError(value: unknown): value is SymphonyApiError {
  return value instanceof SymphonyApiError;
}

/** Strip the `code: ` prefix the Rust error `Display` impl adds, for presentation. */
export function errorMessage(error: unknown): string {
  if (error instanceof SymphonyApiError) {
    const prefix = `${error.code}: `;
    return error.message.startsWith(prefix) ? error.message.slice(prefix.length) : error.message;
  }
  if (error instanceof Error) return error.message;
  return String(error);
}
