import React from "react";
import { NavLink, useNavigate } from "react-router-dom";
import {
  Button,
  CounterBadge,
  Menu,
  MenuDivider,
  MenuItem,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Tooltip
} from "@fluentui/react-components";
import { ChevronsLeft, ChevronsRight, ChevronsUpDown, FolderGit2, Plus } from "lucide-react";
import { cn } from "@symphony/ui";
import { NAV_ITEMS, type NavKey } from "../app/routes.js";
import { shellMessages } from "../app/messages.js";
import { useMessages } from "../lib/i18n.js";
import { useApprovals, useProjects, useRuntimeState } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";

const ProjectSwitcher: React.FC<{ collapsed: boolean }> = ({ collapsed }) => {
  const m = useMessages(shellMessages);
  const projects = useProjects();
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);
  const active = projects.data?.find((p) => p.id === activeProjectId);
  const label = active?.name ?? m.project.all;
  const trigger = (
    <Button
      appearance="subtle"
      aria-label={`${m.project.switch}: ${label}`}
      className="!w-full !justify-start !min-w-0"
      icon={<FolderGit2 className="size-4" aria-hidden />}
    >
      {collapsed ? null : (
        <span className="flex min-w-0 flex-1 items-center gap-1">
          <span className="truncate text-start text-sm">{label}</span>
          <ChevronsUpDown className="ms-auto size-3.5 shrink-0 opacity-60" aria-hidden />
        </span>
      )}
    </Button>
  );
  return (
    <Menu
      checkedValues={{ project: [activeProjectId ?? "__all"] }}
      onCheckedValueChange={(_, data) => {
        const v = data.checkedItems[0];
        setActiveProject(v && v !== "__all" ? v : null);
      }}
    >
      <MenuTrigger disableButtonEnhancement>
        {collapsed ? (
          <Tooltip content={label} relationship="label" positioning="after">
            {trigger}
          </Tooltip>
        ) : (
          trigger
        )}
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          <MenuItemRadio name="project" value="__all">
            {m.project.all}
          </MenuItemRadio>
          {(projects.data ?? []).map((p) => (
            <MenuItemRadio key={p.id} name="project" value={p.id}>
              {p.name}
            </MenuItemRadio>
          ))}
          <MenuDivider />
          <MenuItem icon={<Plus className="size-4" />} onClick={() => setAddProjectOpen(true)}>
            {m.project.add}
          </MenuItem>
        </MenuList>
      </MenuPopover>
    </Menu>
  );
};

/** Collapsible primary navigation. Collapsed state is persisted; tooltips label icons when collapsed. */
export const Sidebar: React.FC = () => {
  const m = useMessages(shellMessages);
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const setCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const state = useRuntimeState();
  const approvals = useApprovals(true);
  const navigate = useNavigate();
  const pendingApprovals = approvals.data?.length ?? 0;

  // Runs show pending approvals (needs attention) first, otherwise the number of live sessions.
  const badges: Partial<Record<NavKey, { count: number; important: boolean }>> = {
    runs: pendingApprovals > 0 ? { count: pendingApprovals, important: true } : { count: state.data?.counts.running ?? 0, important: false },
    reviews: { count: state.data?.counts.human_review ?? 0, important: false }
  };

  return (
    <nav
      aria-label={m.nav.label}
      className={cn(
        "flex h-full shrink-0 flex-col gap-1 border-e border-border bg-surface px-1.5 py-2 transition-[width] duration-150",
        collapsed ? "w-12" : "w-52"
      )}
    >
      <ProjectSwitcher collapsed={collapsed} />
      <div className="my-1 h-px bg-border" />
      <ul className="flex flex-1 flex-col gap-0.5" role="list">
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const label = m.nav[item.key];
          const badge = badges[item.key]?.count ?? 0;
          const link = (
            <NavLink
              to={item.path}
              onKeyDown={(e) => {
                if (e.key === "Enter") navigate(item.path);
              }}
              className={({ isActive }) =>
                cn(
                  "relative flex h-8 items-center gap-2.5 rounded-md px-2 text-sm text-muted-foreground outline-none transition-colors hover:bg-surface-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
                  isActive && "bg-surface-selected font-medium text-foreground before:absolute before:inset-y-1.5 before:start-0 before:w-0.5 before:rounded-full before:bg-accent"
                )
              }
              aria-label={collapsed ? label : undefined}
            >
              <Icon className="size-4 shrink-0" aria-hidden />
              {collapsed ? null : <span className="flex-1 truncate">{label}</span>}
              {badge > 0 ? (
                <CounterBadge
                  count={badge}
                  size="small"
                  appearance="filled"
                  color={badges[item.key]?.important ? "important" : "brand"}
                  className={collapsed ? "!absolute -end-0.5 -top-0.5" : undefined}
                  aria-label={String(badge)}
                />
              ) : null}
            </NavLink>
          );
          return (
            <li key={item.key}>
              {collapsed ? (
                <Tooltip content={label} relationship="description" positioning="after">
                  {link}
                </Tooltip>
              ) : (
                link
              )}
            </li>
          );
        })}
      </ul>
      <Tooltip content={collapsed ? m.nav.expand : m.nav.collapse} relationship="label" positioning="after">
        <Button
          appearance="subtle"
          icon={collapsed ? <ChevronsRight className="size-4" /> : <ChevronsLeft className="size-4" />}
          onClick={() => setCollapsed(!collapsed)}
          aria-expanded={!collapsed}
        />
      </Tooltip>
    </nav>
  );
};
