import React from "react";
import { NavLink, useNavigate } from "react-router-dom";
import { ChevronDown, PanelLeftClose, PanelLeftOpen, Plus } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import { Button, Hint, cn } from "@symphony/ui";
import { NAV_ITEMS, paths, type NavKey } from "../app/routes.js";
import { STAGE_STATUSES, useOpenWorkflowStage, type FlowStage } from "../components/WorkflowStages.js";
import { shellMessages } from "../app/messages.js";
import { useMessages } from "../lib/i18n.js";
import { useApprovals, useRuntimeState, useTasks } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";
import { workbenchMessages } from "./messages.js";
import { ThreadList } from "./ThreadList.js";

const Count: React.FC<{ count: number; important: boolean; floating?: boolean }> = ({ count, important, floating }) => (
  <span
    aria-label={String(count)}
    className={cn(
      "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full px-1 text-[10px] font-semibold tabular-nums",
      important ? "bg-warning text-background" : "bg-accent-subtle text-accent",
      floating && "absolute -end-0.5 -top-0.5"
    )}
  >
    {count}
  </span>
);

/** The Symphony mark: three bars of a score, on the accent colour. */
export const BrandMark: React.FC<{ className?: string }> = ({ className }) => (
  <span aria-hidden className={cn("grid size-7 shrink-0 place-items-center rounded-[9px] bg-accent text-accent-foreground shadow-card", className)}>
    <svg viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <path d="M3.5 6.5v3M8 3.5v9M12.5 5.5v5" />
    </svg>
  </span>
);

type PulseTone = "accent" | "warning" | "info" | "neutral";

const PULSE_TONE: Record<PulseTone, string> = {
  accent: "text-accent",
  warning: "text-warning",
  info: "text-info",
  neutral: "text-foreground"
};

/**
 * The workflow at a glance: how much work sits in each stage that matters right now. Every cell
 * opens the view where that stage is handled.
 */
const WorkflowPulse: React.FC = () => {
  const w = useMessages(workbenchMessages);
  const openStage = useOpenWorkflowStage();
  const state = useRuntimeState();
  const approvals = useApprovals(true);
  const tasks = useTasks(null);
  const counts = state.data?.counts;
  // Queued counts the same statuses as the Home pipeline and the Tasks view it opens (backlog included).
  const queued = tasks.data ? tasks.data.filter((t) => STAGE_STATUSES.queued.includes(t.status)).length : null;

  const cells: { key: FlowStage; label: string; count: number | null; tone: PulseTone }[] = [
    { key: "working", label: w.flow.working, count: counts?.running ?? null, tone: "accent" },
    { key: "needsYou", label: w.flow.needsYou, count: approvals.data ? approvals.data.length : null, tone: "warning" },
    { key: "review", label: w.flow.review, count: counts?.human_review ?? null, tone: "info" },
    { key: "queued", label: w.flow.queued, count: queued, tone: "neutral" }
  ];

  return (
    <div role="group" aria-label={w.flow.label} className="grid grid-cols-4 gap-1 rounded-xl border border-border bg-surface p-1 shadow-card">
      {cells.map((cell) => {
        const active = (cell.count ?? 0) > 0;
        return (
          <button
            key={cell.key}
            type="button"
            onClick={() => openStage(cell.key)}
            aria-label={formatMessage(w.flow.stage, { label: cell.label, count: cell.count ?? "–" })}
            className="flex min-w-0 flex-col items-center gap-0.5 rounded-lg px-1 py-1.5 outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/60"
          >
            <span aria-hidden className={cn("font-display text-base font-semibold leading-none tabular-nums", active ? PULSE_TONE[cell.tone] : "text-subtle-foreground")}>
              {cell.count ?? "–"}
            </span>
            <span aria-hidden className="max-w-full truncate text-[10px] leading-tight text-muted-foreground">
              {cell.label}
            </span>
          </button>
        );
      })}
    </div>
  );
};

/**
 * Thread-first sidebar on the window canvas: new task, the workflow at a glance, threads grouped
 * by project, and the other views under "Manage". Collapsed it is an icon rail with tooltips.
 */
export const Sidebar: React.FC = () => {
  const m = useMessages(shellMessages);
  const w = useMessages(workbenchMessages);
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const setCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const manageOpen = useUiStore((s) => s.manageOpen);
  const setManageOpen = useUiStore((s) => s.setManageOpen);
  const state = useRuntimeState();
  const approvals = useApprovals(true);
  const navigate = useNavigate();
  const pendingApprovals = approvals.data?.length ?? 0;

  // Tasks show pending approvals (needs attention) first, otherwise the number of live sessions.
  const badges: Partial<Record<NavKey, { count: number; important: boolean }>> = {
    tasks: pendingApprovals > 0 ? { count: pendingApprovals, important: true } : { count: state.data?.counts.running ?? 0, important: false },
    reviews: { count: state.data?.counts.human_review ?? 0, important: false }
  };

  const items = (
    <ul className="flex flex-col gap-px" role="list">
      {NAV_ITEMS.map((item) => {
        const Icon = item.icon;
        const label = m.nav[item.key];
        const badge = badges[item.key]?.count ?? 0;
        const link = (
          <NavLink
            to={item.path}
            className={({ isActive }) =>
              cn(
                "relative flex h-8 items-center gap-2.5 rounded-lg px-2 text-sm text-muted-foreground outline-none transition-colors hover:bg-foreground/5 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60 [@media(max-height:760px)]:h-7",
                collapsed && "justify-center px-0",
                isActive && "bg-surface font-medium text-foreground shadow-card hover:bg-surface"
              )
            }
            aria-label={collapsed ? label : undefined}
          >
            <Icon className="size-4 shrink-0" aria-hidden />
            {collapsed ? null : <span className="flex-1 truncate">{label}</span>}
            {badge > 0 ? <Count count={badge} important={badges[item.key]?.important ?? false} floating={collapsed} /> : null}
          </NavLink>
        );
        const startsSetup = item.group === "setup" && NAV_ITEMS[NAV_ITEMS.indexOf(item) - 1]?.group === "work";
        return (
          <li key={item.key} className={cn(startsSetup && "mt-1.5 border-t border-border pt-1.5")}>
            {collapsed ? (
              <Hint label={label} side="right">
                {link}
              </Hint>
            ) : (
              link
            )}
          </li>
        );
      })}
    </ul>
  );

  return (
    <nav
      aria-label={m.nav.label}
      className={cn("flex h-full shrink-0 flex-col transition-[width] duration-200 ease-out", collapsed ? "w-14 items-stretch px-2 py-2.5" : "w-[17.5rem]")}
    >
      <div className={cn("flex shrink-0 items-center gap-2.5", collapsed ? "mb-2.5 flex-col" : "px-3.5 pb-3 pt-3.5")}>
        <BrandMark />
        {collapsed ? null : <span className="flex-1 font-display text-[15px] font-semibold tracking-tight text-foreground">Symphony</span>}
        <Hint label={collapsed ? m.nav.expand : m.nav.collapse} side="right">
          <Button variant="subtle" size="icon-sm" onClick={() => setCollapsed(!collapsed)} aria-expanded={!collapsed} aria-label={collapsed ? m.nav.expand : m.nav.collapse}>
            {collapsed ? <PanelLeftOpen /> : <PanelLeftClose />}
          </Button>
        </Hint>
      </div>

      <div className={cn("shrink-0", collapsed ? "mb-2.5" : "space-y-2.5 px-3 pb-3")}>
        {collapsed ? (
          <Hint label={w.sidebar.newTask} side="right">
            <Button variant="default" size="icon" className="w-full" aria-label={w.sidebar.newTask} onClick={() => navigate(paths.home())}>
              <Plus />
            </Button>
          </Hint>
        ) : (
          <>
            <Button variant="default" className="h-9 w-full justify-start gap-2 rounded-xl px-3" onClick={() => navigate(paths.home())}>
              <Plus />
              <span className="flex-1 text-start">{w.sidebar.newTask}</span>
              <kbd aria-hidden className="font-mono text-[10px] font-normal opacity-70">
                {m.shortcuts.newTask}
              </kbd>
            </Button>
            <WorkflowPulse />
          </>
        )}
      </div>

      {collapsed ? (
        <div className="flex-1 overflow-y-auto">{items}</div>
      ) : (
        <>
          <ThreadList />
          <div className="shrink-0 border-t border-border px-2.5 pb-1.5 pt-1.5">
            <button
              type="button"
              onClick={() => setManageOpen(!manageOpen)}
              aria-expanded={manageOpen}
              className="flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-[11px] font-semibold uppercase tracking-wider text-subtle-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
            >
              <span className="flex-1 text-start">{w.sidebar.manage}</span>
              <ChevronDown className={cn("size-3.5 transition-transform duration-200", !manageOpen && "-rotate-90")} aria-hidden />
            </button>
            {manageOpen ? <div className="max-h-[min(56vh,calc(100vh_-_27rem))] overflow-y-auto">{items}</div> : null}
          </div>
        </>
      )}
    </nav>
  );
};
