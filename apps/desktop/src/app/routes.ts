import {
  Activity,
  Home,
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
  | "activity"
  | "worktrees"
  | "reviews"
  | "workflow"
  | "providers"
  | "settings";

export interface NavItem {
  key: NavKey;
  path: string;
  icon: LucideIcon;
  /** `work` is what you look at day to day; `setup` is configuration and inspection. */
  group: "work" | "setup";
}

/** Durable application locations (hash routes, restorable across launches). */
export const NAV_ITEMS: ReadonlyArray<NavItem> = [
  { key: "overview", path: "/overview", icon: LayoutDashboard, group: "work" },
  { key: "projects", path: "/projects", icon: FolderGit2, group: "work" },
  { key: "tasks", path: "/tasks", icon: KanbanSquare, group: "work" },
  { key: "reviews", path: "/reviews", icon: GitPullRequest, group: "work" },
  { key: "activity", path: "/activity", icon: Activity, group: "work" },
  { key: "worktrees", path: "/worktrees", icon: GitBranch, group: "setup" },
  { key: "workflow", path: "/workflow", icon: ScrollText, group: "setup" },
  { key: "providers", path: "/providers", icon: Plug, group: "setup" },
  { key: "settings", path: "/settings", icon: Settings, group: "setup" }
];

export const paths = {
  home: () => "/",
  activity: () => "/activity",
  overview: () => "/overview",
  projects: () => "/projects",
  project: (id: string) => `/projects/${encodeURIComponent(id)}`,
  tasks: () => "/tasks",
  /** The Runs view of the Tasks page (what `/runs` used to be). */
  runs: () => "/tasks?view=runs",
  run: (taskId: string) => `/runs/${encodeURIComponent(taskId)}`,
  worktrees: () => "/worktrees",
  reviews: () => "/reviews",
  review: (id: string) => `/reviews/${encodeURIComponent(id)}`,
  workflow: () => "/workflow",
  providers: () => "/providers",
  settings: () => "/settings"
};

export const HomeIcon = Home;
