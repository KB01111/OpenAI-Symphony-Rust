import {
  Activity,
  Bot,
  FolderGit2,
  GitBranch,
  GitPullRequest,
  KanbanSquare,
  LayoutDashboard,
  Plug,
  ScrollText,
  Settings,
  type LucideIcon
} from "lucide-react";

export type NavKey =
  | "overview"
  | "projects"
  | "tasks"
  | "runs"
  | "worktrees"
  | "reviews"
  | "workflow"
  | "providers"
  | "settings";

export interface NavItem {
  key: NavKey;
  path: string;
  icon: LucideIcon;
}

/** Durable application locations (hash routes, restorable across launches). */
export const NAV_ITEMS: ReadonlyArray<NavItem> = [
  { key: "overview", path: "/overview", icon: LayoutDashboard },
  { key: "projects", path: "/projects", icon: FolderGit2 },
  { key: "tasks", path: "/tasks", icon: KanbanSquare },
  { key: "runs", path: "/runs", icon: Bot },
  { key: "worktrees", path: "/worktrees", icon: GitBranch },
  { key: "reviews", path: "/reviews", icon: GitPullRequest },
  { key: "workflow", path: "/workflow", icon: ScrollText },
  { key: "providers", path: "/providers", icon: Plug },
  { key: "settings", path: "/settings", icon: Settings }
];

export const paths = {
  overview: () => "/overview",
  projects: () => "/projects",
  project: (id: string) => `/projects/${encodeURIComponent(id)}`,
  tasks: () => "/tasks",
  runs: () => "/runs",
  run: (taskId: string) => `/runs/${encodeURIComponent(taskId)}`,
  worktrees: () => "/worktrees",
  reviews: () => "/reviews",
  review: (id: string) => `/reviews/${encodeURIComponent(id)}`,
  workflow: () => "/workflow",
  providers: () => "/providers",
  settings: () => "/settings"
};

export const ActivityIcon = Activity;
