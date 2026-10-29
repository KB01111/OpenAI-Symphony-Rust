import React from "react";
import { NavLink, useNavigate } from "react-router-dom";
import { ChevronDown, ChevronsLeft, ChevronsRight, Plus } from "lucide-react";
import { Button, Hint, cn } from "@symphony/ui";
import { NAV_ITEMS, paths, type NavKey } from "../app/routes.js";
import { shellMessages } from "../app/messages.js";
import { useMessages } from "../lib/i18n.js";
import { useApprovals, useRuntimeState } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";
import { workbenchMessages } from "./messages.js";
import { ThreadList } from "./ThreadList.js";

const Count: React.FC<{ count: number; important: boolean; floating?: boolean }> = ({ count, important, floating }) => (
  <span
    aria-label={String(count)}
    className={cn(
      "inline-flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] font-semibold tabular-nums",
      important ? "bg-warning text-background" : "bg-accent-subtle text-accent",
      floating && "absolute -end-0.5 -top-0.5"
    )}
  >
    {count}
  </span>
);

/**
 * Thread-first sidebar: new task, threads grouped by project, and the secondary views under
 * "Manage". Collapsed state is persisted; collapsed it is an icon rail with tooltips.
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

  // Runs show pending approvals (needs attention) first, otherwise the number of live sessions.
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
                "relative flex h-8 items-center gap-2.5 rounded-lg px-2 text-sm [@media(max-height:760px)]:h-7 text-muted-foreground outline-none transition-colors hover:bg-surface-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60",
                isActive && "bg-surface-selected font-medium text-foreground"
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
          <li key={item.key} className={cn(startsSetup && "mt-1 border-t border-border pt-1")}>
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
      className={cn(
        "flex h-full shrink-0 flex-col border-e border-border bg-surface transition-[width] duration-150",
        collapsed ? "w-14 items-stretch px-2 py-2" : "w-72"
      )}
    >
      <div className={cn("flex shrink-0 items-center gap-2", collapsed ? "mb-2 flex-col" : "px-3 pb-2 pt-3")}>
        <span aria-hidden className="grid size-7 shrink-0 place-items-center rounded-lg bg-accent font-display text-sm font-bold text-accent-foreground">
          S
        </span>
        {collapsed ? null : <span className="flex-1 font-display text-sm font-semibold text-foreground">Symphony</span>}
        <Hint label={collapsed ? m.nav.expand : m.nav.collapse} side="right">
          <Button variant="subtle" size="icon-sm" onClick={() => setCollapsed(!collapsed)} aria-expanded={!collapsed} aria-label={collapsed ? m.nav.expand : m.nav.collapse}>
            {collapsed ? <ChevronsRight /> : <ChevronsLeft />}
          </Button>
        </Hint>
      </div>

      <div className={cn("shrink-0", collapsed ? "mb-2" : "px-3 pb-2")}>
        {collapsed ? (
          <Hint label={w.sidebar.newTask} side="right">
            <Button variant="default" size="icon" className="w-full" aria-label={w.sidebar.newTask} onClick={() => navigate(paths.home())}>
              <Plus />
            </Button>
          </Hint>
        ) : (
          <Button variant="default" className="w-full justify-start rounded-lg" onClick={() => navigate(paths.home())}>
            <Plus />
            {w.sidebar.newTask}
          </Button>
        )}
      </div>

      {collapsed ? <div className="flex-1 overflow-y-auto">{items}</div> : (
        <>
          <ThreadList />
          <div className="shrink-0 border-t border-border px-2 pb-2 pt-1">
            <button
              type="button"
              onClick={() => setManageOpen(!manageOpen)}
              aria-expanded={manageOpen}
              className="flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
            >
              <ChevronDown className={cn("size-3.5 transition-transform", !manageOpen && "-rotate-90")} aria-hidden />
              {w.sidebar.manage}
            </button>
            {manageOpen ? <div className="max-h-[min(56vh,calc(100vh_-_18rem))] overflow-y-auto">{items}</div> : null}
          </div>
        </>
      )}
    </nav>
  );
};
