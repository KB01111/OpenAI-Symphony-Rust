// Providers & theme
export * from "./providers/SymphonyDesignProvider.js";
export { symphonyDarkTheme, symphonyLightTheme } from "./theme/fluentThemes.js";
export { uiMessages, useUiStrings } from "./i18n.js";
export { cn } from "./lib/utils.js";

// AI-native components (OpenAI Apps SDK UI)
export * from "./ai/badges.js";
export * from "./ai/eventFormat.js";
export * from "./ai/AiApprovalBanner.js";
export * from "./ai/AiEventStreamCard.js";
export * from "./ai/MergeReadinessCard.js";
export * from "./ai/misc.js";

// Feedback (Fluent UI)
export * from "./feedback/index.js";

// Data display & layout
export * from "./data-display/index.js";
export * from "./data-display/DataTable.js";
export * from "./layouts/index.js";

// shadcn/ui primitives (advanced workspace components)
export * from "./primitives/resizable.js";
export * from "./primitives/command.js";
export * from "./primitives/scroll-area.js";
export * from "./primitives/context-menu.js";
export * from "./primitives/sheet.js";
export * from "./primitives/skeleton.js";
export * from "./primitives/table.js";

// Apps SDK UI building blocks used directly by feature views
export { Badge as SdkBadge } from "@openai/apps-sdk-ui/components/Badge";
export { Button as SdkButton, CopyButton as SdkCopyButton } from "@openai/apps-sdk-ui/components/Button";
export { CodeBlock as SdkCodeBlock } from "@openai/apps-sdk-ui/components/CodeBlock";
export { Markdown as SdkMarkdown } from "@openai/apps-sdk-ui/components/Markdown";
export { Alert as SdkAlert } from "@openai/apps-sdk-ui/components/Alert";
export { LoadingIndicator as SdkLoadingIndicator } from "@openai/apps-sdk-ui/components/Indicator";
