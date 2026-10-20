# Symphony Design System

The design system lives in `packages/ui` and is shared by the desktop app and the mobile companion.
It combines three component libraries under **one token source** and **one owner per component category**.

```
packages/ui/src/
  tokens/tokens.css        single source of colour, typography, radius, density tokens
  theme/fluentThemes.ts    Fluent UI v9 themes whose values reference the CSS tokens
  providers/               SymphonyDesignProvider (theme, density, motion, locale, toaster)
  primitives/              shadcn/ui components (generated with the shadcn CLI)
  ai/                      AI-native components built on OpenAI Apps SDK UI
  feedback/                confirm dialog, error state, loading rows, toasts, live announcer
  data-display/            metric tile, key/value list, time & duration, virtual list, data table, telemetry
  layouts/                 page header, panel, status dot
  i18n.ts                  strings owned by shared components (EN/SV)
  index.ts                 public entry point
```

## 1. Tokens

`tokens.css` defines semantic variables for light and dark themes, selected by `<html data-theme>`:

| Token | Purpose |
| --- | --- |
| `--background` | App canvas behind panels |
| `--foreground` | Primary text |
| `--surface`, `--surface-raised`, `--surface-sunken` | Panels, popovers, inset wells |
| `--surface-hover`, `--surface-selected` | Interactive states |
| `--border`, `--border-strong` | Dividers and control outlines |
| `--muted`, `--muted-foreground`, `--subtle-foreground` | Secondary surfaces and text |
| `--accent`, `--accent-hover`, `--accent-foreground`, `--accent-subtle` | Restrained blue for active actions |
| `--destructive`, `--warning`, `--success`, `--info` (+ `-subtle`) | Semantic status only |
| `--ring` | Focus ring |
| `--code-background`, `--diff-add`, `--diff-remove` | Code and diff surfaces |
| `--font-sans` | Segoe UI Variable Text → Segoe UI → system |
| `--font-display` | Segoe UI Variable Display |
| `--font-mono` | Cascadia Code → Cascadia Mono → Consolas |
| `--radius` | 6 px base radius |
| `--density-*` | Row/control height, gaps and base font size |

The same variables feed every library:

- **Tailwind CSS v4** — `@theme inline` maps them to utilities (`bg-surface`, `text-muted-foreground`,
  `border-border`, `bg-warning-subtle`, `font-mono`, `h-row`, …). Feature code uses only these classes;
  hard-coded hex or palette colours are not allowed outside `tokens.css`.
- **shadcn/ui** — the shadcn semantic names (`--card`, `--popover`, `--primary`, `--input`, `--ring`) are
  aliases of the Symphony tokens.
- **Fluent UI v9** — `fluentThemes.ts` derives brand ramps with `createDarkTheme/createLightTheme` and then
  points surface, text, stroke, brand and status tokens at `var(--…)`. Fluent therefore switches theme
  together with everything else and portals inherit it (`applyStylesToPortals`).
- **OpenAI Apps SDK UI** — `applyDocumentTheme(theme)` sets the same `data-theme` attribute the tokens use.

The palette is a graphite neutral scale with a single restrained blue accent. Semantic green, amber and
red are used for status only. No gradients or glass effects are used; elevation is a subtle border plus
`--shadow-elevated` on floating surfaces.

## 2. `SymphonyDesignProvider`

```tsx
<SymphonyDesignProvider themeMode="system" density="compact" motion="system" locale="sv">
  <App />
</SymphonyDesignProvider>
```

- Resolves `system` against `prefers-color-scheme` (Windows light/dark setting) and follows live changes.
- Writes `data-theme`, `data-density`, `data-motion` and `lang` on `<html>`.
- Provides the matching Fluent theme and wraps children in `AppsSDKUIProvider`.
- Mounts the single app-wide Fluent `Toaster`; `useNotify()` dispatches toasts.
- Exposes `useDesign()` (resolved theme, density, reduced motion, locale) and `useUiStrings()`.

Reduced motion: when the OS requests it (or `motion="reduced"`), CSS animations and transitions are
neutralised globally by `tokens.css`. Components avoid JavaScript animation loops.

Density: **compact** (default, IDE-like: 28 px rows, 13 px text) and **comfortable** (36 px rows, 14 px text).
The density font is applied to `<body>`, not `<html>`: the root font size stays at 16 px so `rem`-based
spacing (Tailwind, Apps SDK) matches Fluent's px-based controls. Tailwind's `text-sm`/`text-xs` and Fluent's
`fontSizeBase300`/`fontSizeBase200` all resolve to `--density-font`/`--density-font-xs`, so body and caption
text are the same size whichever library renders them.

### Cascade layers

Apps SDK UI ships component CSS in `@layer components`. Each app imports its global stylesheet **first**
(`main.tsx`) and that stylesheet starts with `@layer theme, base, components, utilities;`, so the layer order
is fixed before any library CSS is injected. Otherwise `components` can be ordered below Tailwind's preflight
(`base`) and Apps SDK buttons lose their padding, radius and colours.

## 3. Component ownership

Exactly one library owns each interaction category. Competing implementations are not rendered.

| Category | Owner | Components |
| --- | --- | --- |
| App chrome & commands | Fluent UI v9 | `Toolbar`, `ToolbarButton`, `Button`, `Menu`/`MenuButton`, `CounterBadge`, `Divider` |
| Tooltips | Fluent UI v9 | `Tooltip` (Apps SDK `Tooltip` is not used) |
| Dialogs & confirmations | Fluent UI v9 | `Dialog` everywhere, `ConfirmDialog`; the command palette hosts shadcn `Command` inside a Fluent `Dialog` (shadcn `dialog` was intentionally removed) |
| Forms | Fluent UI v9 | `Field`, `Input`, `Textarea`, `Dropdown`, `Combobox`, `Switch`, `SpinButton`, `Checkbox`, `RadioGroup` |
| Status & progress | Fluent UI v9 | `MessageBar`, `Spinner`, `ProgressBar`, toasts |
| Tabs | Fluent UI v9 | `TabList`/`Tab` |
| Workspace layout | shadcn/ui | `Resizable*` (react-resizable-panels v4, persisted with `useDefaultLayout`) |
| Command palette | shadcn/ui | `Command*` (cmdk) |
| Context menus | shadcn/ui | `ContextMenu*` (right-click on rows and cards) |
| Data tables | shadcn/ui + TanStack | `Table` primitives + `DataTable` (sorting, keyboard activation, windowing) |
| Scroll areas, sheets, skeletons | shadcn/ui | `ScrollArea`, `Sheet`, `Skeleton` |
| AI-native surfaces | OpenAI Apps SDK UI | `AiApprovalBanner`, `AiEventStreamCard`, `MergeReadinessCard`, `TaskStatusBadge`, `PhaseBadge`, `ProviderBadge`, `ApprovalStatusBadge`, `AiEmptyView`, `PromptComposerBox`, `LogModeSwitch`, `StreamingText`; `CodeBlock`, `Markdown`, `CopyButton`, `Badge`, `Alert`, `ShimmerText`, `EmptyMessage`, `SegmentedControl`, `Textarea` used inside them |

shadcn components are installed with the shadcn CLI into `packages/ui/src/primitives` using
`packages/ui/components.json` (style `new-york`, Tailwind v4 CSS variables, lucide icons). To add one:

```bash
cd packages/ui && pnpm dlx shadcn@latest add <component>
```

then make sure generated imports use `../lib/utils` and that it does not duplicate a Fluent-owned category.

Apps SDK UI is used as a **component library only**; nothing depends on ChatGPT host globals.

## 4. AI components

| Component | Purpose |
| --- | --- |
| `AiApprovalBanner` | Shows a pending command/file-change approval with the four runtime decisions (approve once, approve for session, decline, abort turn). Locks itself while a decision is in flight to prevent duplicate submissions. |
| `ConfirmDialog` (feedback) | Fluent alert dialog for destructive actions. Fires `onConfirm` once per opening (a second click in the same frame is ignored) and stays locked while `pending`. |
| `AiEventStreamCard` | One timeline entry. Uses `formatAgentEvent` to render agent messages (Markdown), command executions (code + exit code + output), file changes (paths + diffs), tool calls, approvals, errors and lifecycle events. Concise vs technical mode (event types, raw JSON-RPC payload). Memoized per event so streaming never re-renders existing entries. |
| `MergeReadinessCard` | Evidence-backed 1–5 score with CI / validation / conflicts / security / reviewed comments breakdown, blockers and evidence. Approval is a separate explicit action and never merges. |
| `TaskStatusBadge`, `PhaseBadge`, `ProviderBadge` | Localized status chips; active phases shimmer (static under reduced motion). |
| `AiEmptyView`, `PromptComposerBox`, `LogModeSwitch`, `StreamingText` | Agent empty states, prompt composition (Ctrl+Enter submits), log verbosity, live activity text. |

## 5. Accessibility

- Focus is always visible (`:focus-visible` ring on `--ring`).
- Icon-only buttons carry labels through Fluent `Tooltip relationship="label"` or `aria-label`.
- Dialog focus management comes from Fluent `Dialog`; the palette is keyboard-first (cmdk).
- Data tables expose `aria-sort`, rows are focusable and activate with Enter/Space.
- `LiveAnnouncer` provides polite/assertive live regions; toasts announce with matching politeness.
- Colour is never the only signal: status chips have text, readiness has a numeric score and `role="meter"`.

## 6. Localization

Shared strings live in `packages/shared-types/src/i18n` (`en`, `sv`). Components in this package use
`useUiStrings()`. Feature modules declare their own catalogues with `defineMessages({ en, sv })`, where the
Swedish catalogue must have exactly the English shape (enforced by TypeScript).
