# Symphony Design System

The design system lives in `packages/ui` and is shared by the desktop app and the mobile companion.
It is **one component system**: shadcn/ui primitives (Radix UI + Tailwind CSS v4) driven by **one token source**.
There is no second control library on the desktop: Fluent UI and the OpenAI Apps SDK UI were removed because
three libraries with different sizing, breakpoints and CSS layers produced overlapping and clipped controls.

```
packages/ui/src/
  tokens/tokens.css        single source of colour, typography, radius, density tokens
  providers/               SymphonyDesignProvider (theme, density, motion, locale, tooltips, toaster)
  primitives/              shadcn/ui-style components on Radix UI (button, input, select, dialog, tabs, …)
  ai/                      AI-native components (approval banner, event stream card, readiness card, badges)
  feedback/                confirm dialog, error state, loading rows, toasts (sonner), live announcer
  data-display/            metric tile, key/value list, time & duration, virtual list, data table, telemetry
  layouts/                 page header, panel, status dot
  legacy.tsx               Fluent theme bridge used only by the mobile companion
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
| `--overlay` | Modal and sheet scrim |
| `--code-background`, `--diff-add`, `--diff-remove` | Code and diff surfaces |
| `--font-sans`, `--font-display`, `--font-mono` | Segoe UI Variable stack, Cascadia mono |
| `--radius` | 8 px base radius |
| `--density-*` | Row/control height, gaps and base font size |

Tailwind CSS v4 reads them through `@theme inline` (`bg-surface`, `text-muted-foreground`, `border-border`,
`bg-warning-subtle`, `font-mono`, `h-row`, …). Feature code uses only these classes; hard-coded hex or palette
colours are not allowed outside `tokens.css`. The shadcn semantic names (`--card`, `--popover`, `--primary`,
`--input`, `--ring`) are aliases of the Symphony tokens.

The palette is a graphite neutral scale with a single restrained blue accent. Semantic green, amber and red are
used for status only. No gradients or glass effects are used; elevation is a subtle border plus
`--shadow-elevated` on floating surfaces.

## 2. `SymphonyDesignProvider`

```tsx
<SymphonyDesignProvider themeMode="system" density="compact" motion="system" locale="sv">
  <App />
</SymphonyDesignProvider>
```

- Resolves `system` against `prefers-color-scheme` and follows live changes.
- Writes `data-theme`, `data-density`, `data-motion` and `lang` on `<html>`.
- Provides the tooltip context and mounts the single app-wide `sonner` toaster; `useNotify()` dispatches toasts.
- Exposes `useDesign()` (resolved theme, density, reduced motion, locale) and `useUiStrings()`.

Reduced motion: when the OS requests it (or `motion="reduced"`), CSS animations and transitions are neutralised
globally by `tokens.css`.

Density: **compact** (default, IDE-like: 28 px rows, 13 px text) and **comfortable** (36 px rows, 14 px text).
Tailwind's `text-sm`/`text-xs` resolve to `--density-font`/`--density-font-xs`.

### Cascade layers

The app's global stylesheet starts with `@layer theme, base, components, utilities;` and imports Tailwind and
`tokens.css`. There is no third-party component CSS, so the layer order cannot be disturbed.

## 3. Layout rules (these prevent the overlap and clipping bugs)

1. **Container queries, not viewport breakpoints.** A form or table inside a side sheet or a resizable pane is
   narrower than the window. Use `@container` on the parent and `@md:`/`@3xl:` variants (`FieldGrid`,
   `DataTable`, settings sections). `sm:`/`md:`/`lg:` are only for shell-level decisions.
2. **Controls never impose a minimum width.** `Input`, `Select`, `Combobox` and `MultiSelect` are `w-full min-w-0`;
   the grid cell decides their width.
3. **One scroller per region.** `DataTable` owns the only scroll container of a table. Low-priority columns hide by
   the table's own width (`meta: { className: "hidden @3xl:table-cell" }`) instead of scrolling sideways.
4. **Detail lives in a `Sheet`, not beside a table.** Master/detail views (Projects) open the detail in a side
   sheet so the list keeps its full width.
5. **Text wraps or truncates inside its box.** Header meta rows use `flex-wrap`, never `shrink-0`.

## 4. Components

| Category | Components |
| --- | --- |
| Actions | `Button` (default, secondary, outline, ghost, subtle, destructive, link; sm/default/lg/icon), `ToggleGroup` (segmented control), `DropdownMenu`, `ContextMenu` |
| Forms | `Field` (label, hint, error, `group` for radio groups), `FieldGrid`, `FormSection`, `Input` (with `icon`), `Textarea`, `Select`, `Combobox`, `MultiSelect`, `Switch`, `Checkbox`, `RadioGroup`/`RadioOption`, `Label` |
| Overlays | `Dialog`, `ConfirmDialog` (alert dialog), `Sheet`, `Popover`, `Tooltip`/`Hint`, command palette (`Command`, cmdk) |
| Feedback | `Alert`, `Badge`, `Spinner`, `Progress`, `Skeleton`, toasts |
| Navigation | `Tabs` |
| Data | `Table`, `DataTable` (TanStack Table + virtualization), `MetricTile`, `KeyValueList`, `VirtualList` |
| Workspace | `ResizablePanelGroup` (react-resizable-panels), `ScrollArea`, `PageHeader`, `Panel` |
| AI surfaces | `AiApprovalBanner`, `AiEventStreamCard`, `MergeReadinessCard`, `TaskStatusBadge`, `PhaseBadge`, `ProviderBadge`, `ApprovalStatusBadge`, `AiEmptyView`, `PromptComposerBox`, `LogModeSwitch`, `StreamingText`, `CodeBlock`, `Markdown` |

`Select` forwards the `id`/`aria-*` that `Field` injects to its trigger (Radix `Select.Root` renders no element),
so a field label always names the select. `Hint` wraps one child with a tooltip; `asLabel` also uses a string
label as the child's accessible name (icon-only buttons).

New shadcn components are added with the shadcn CLI into `packages/ui/src/primitives` using
`packages/ui/components.json` (style `new-york`, Tailwind v4 CSS variables, lucide icons):

```bash
cd packages/ui && pnpm dlx shadcn@latest add <component>
```

Then make sure generated imports use `../lib/utils` and use only Symphony tokens.

## 5. AI components

| Component | Purpose |
| --- | --- |
| `AiApprovalBanner` | Shows a pending command/file-change approval with the four runtime decisions (approve once, approve for session, decline, abort turn). Locks itself while a decision is in flight to prevent duplicate submissions. |
| `ConfirmDialog` (feedback) | Alert dialog for destructive actions. Fires `onConfirm` once per opening and stays locked while `pending`. |
| `AiEventStreamCard` | One timeline entry. Uses `formatAgentEvent` to render agent messages (Markdown), command executions (code + exit code + output), file changes (paths + diffs), tool calls, approvals, errors and lifecycle events. Concise vs technical mode. Memoized per event so streaming never re-renders existing entries. |
| `MergeReadinessCard` | Evidence-backed 1–5 score with CI / validation / conflicts / security / reviewed comments breakdown, blockers and evidence. Approval is a separate explicit action and never merges. |
| `TaskStatusBadge`, `PhaseBadge`, `ProviderBadge` | Localized status chips; active phases pulse (static under reduced motion). |
| `AiEmptyView`, `PromptComposerBox`, `LogModeSwitch`, `StreamingText` | Agent empty states, prompt composition (Ctrl+Enter submits), log verbosity, live activity text. |

`Markdown` renders GitHub-flavoured Markdown with `react-markdown` (raw HTML is never rendered); `CodeBlock`
has a named copy button and scrolls inside itself.

## 6. Accessibility

- Focus is always visible (`:focus-visible` ring on `--ring`).
- Text tokens (`--foreground`, `--muted-foreground`, `--subtle-foreground` and the status colours) keep
  4.5:1 against every surface of their theme, including their own `-subtle` tint.
- Icon-only buttons carry an `aria-label` (use `Hint ... asLabel`).
- Scroll containers without their own controls (diffs, logs, `VirtualList`, code blocks) are focusable.
- Dialog focus management comes from Radix; the palette is keyboard-first (cmdk).
- Tabs activate on mouse-down, focus or a plain click (`TabsTrigger` focuses on click).
- Data tables expose `aria-sort`, rows are focusable and activate with Enter/Space.
- `LiveAnnouncer` provides polite/assertive live regions; toasts announce with matching politeness.
- Colour is never the only signal: status chips have text, readiness has a numeric score and `role="meter"`.

## 7. Localization

Shared strings live in `packages/shared-types/src/i18n` (`en`, `sv`). Components in this package use
`useUiStrings()`. Feature modules declare their own catalogues with `defineMessages({ en, sv })`, where the
Swedish catalogue must have exactly the English shape (enforced by TypeScript).

## 8. Mobile companion

`apps/mobile` still renders some Fluent UI v9 controls. It wraps its tree in `FluentBridge` from
`@symphony/ui/legacy`, which maps the same tokens onto a Fluent theme. The desktop app does not import Fluent.
