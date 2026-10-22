export * from "./errors/index.js";
export * from "./client/SymphonyClient.js";
export * from "./config/apiConfig.js";
export * from "./transport/discovery.js";
export {
  isTauri,
  listenUpdateProgress,
  nativeHost,
  type InstallOutcome,
  type LocalApiConfig,
  type NativeHost,
  type UpdateCheck,
  type UpdateProgress
} from "./transport/tauri.js";
export * from "./streaming/sse.js";
export * from "./streaming/native.js";
export * from "./streaming/batcher.js";
export type * from "@symphony/shared-types";
