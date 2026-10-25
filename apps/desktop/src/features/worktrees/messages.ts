import { defineMessages } from "../../lib/i18n.js";

/** Strings for the Worktrees view (`/worktrees`). */
export const worktreesMessages = defineMessages({
  en: {
    title: "Worktrees",
    description: "{total, plural, one {# worktree} other {# worktrees}} · {dirty} with uncommitted changes",
    filters: "Worktree filters",
    project: "Project",
    allProjects: "All projects",
    dirtyOnly: "Uncommitted changes only",
    tableLabel: "Task worktrees",
    shown: "{shown} of {total}",
    columns: {
      task: "Task",
      project: "Project",
      branch: "Branch",
      path: "Path",
      state: "Changes",
      lastModified: "Last modified",
      status: "Task status",
      actions: "Actions"
    },
    state: {
      dirty: "Uncommitted changes",
      clean: "Clean",
      missing: "Missing on disk",
      activeSession: "Agent session active"
    },
    noBranch: "No branch",
    unknownProject: "Unknown project",
    actions: {
      label: "Actions for {identifier}",
      openExecution: "Open live execution",
      openExplorer: "Open in Explorer",
      viewDiff: "View diff"
    },
    protectedHint: "Worktrees are managed by the runtime and are never deleted from here; uncommitted changes stay protected.",
    diffDialog: {
      title: "Changes in {identifier}",
      description: "{branch} · {path}"
    },
    empty: {
      title: "No worktrees yet",
      description: "Each task gets an isolated git worktree when its agent run starts.",
      cta: "Go to Tasks",
      filtered: "No worktrees match the current filters.",
      clear: "Clear filters"
    },
    loadFailed: "Could not load worktrees"
  },
  sv: {
    title: "Arbetsträd",
    description: "{total} arbetsträd · {dirty} med ändringar som inte har checkats in",
    filters: "Filter för arbetsträd",
    project: "Projekt",
    allProjects: "Alla projekt",
    dirtyOnly: "Endast ändringar som inte har checkats in",
    tableLabel: "Uppgifternas arbetsträd",
    shown: "{shown} av {total}",
    columns: {
      task: "Uppgift",
      project: "Projekt",
      branch: "Gren",
      path: "Sökväg",
      state: "Ändringar",
      lastModified: "Senast ändrad",
      status: "Uppgiftsstatus",
      actions: "Åtgärder"
    },
    state: {
      dirty: "Ändringar som inte har checkats in",
      clean: "Rent",
      missing: "Saknas på disk",
      activeSession: "Agentsession aktiv"
    },
    noBranch: "Ingen gren",
    unknownProject: "Okänt projekt",
    actions: {
      label: "Åtgärder för {identifier}",
      openExecution: "Öppna livekörning",
      openExplorer: "Öppna i Utforskaren",
      viewDiff: "Visa diff"
    },
    protectedHint: "Arbetsträd hanteras av körmiljön och tas aldrig bort härifrån; ändringar som inte har checkats in förblir skyddade.",
    diffDialog: {
      title: "Ändringar i {identifier}",
      description: "{branch} · {path}"
    },
    empty: {
      title: "Inga arbetsträd ännu",
      description: "Varje uppgift får ett isolerat git-arbetsträd när dess agentkörning startar.",
      cta: "Gå till Uppgifter",
      filtered: "Inga arbetsträd matchar de aktuella filtren.",
      clear: "Rensa filter"
    },
    loadFailed: "Det gick inte att läsa in arbetsträd"
  }
});
