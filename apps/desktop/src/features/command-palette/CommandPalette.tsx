import React, { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { FolderGit2, Keyboard, Languages, MoonStar, Pause, Play, Plus, RefreshCw, Rows3, SquareKanban } from "lucide-react";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle
} from "@symphony/ui";
import { NAV_ITEMS, paths } from "../../app/routes.js";
import { shellMessages } from "../../app/messages.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useProjects, useRefreshRuntime, useRuntimeState, useSchedulerToggle, useTasks } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";

/**
 * Ctrl+K command palette: shadcn `Command` (cmdk) for fuzzy search and keyboard navigation,
 * hosted in the app's dialog primitive for focus management.
 */
export const CommandPalette: React.FC = () => {
  const m = useMessages(shellMessages);
  const t = useT();
  const navigate = useNavigate();
  const open = useUiStore((s) => s.paletteOpen);
  const mode = useUiStore((s) => s.paletteMode);
  const close = useUiStore((s) => s.closePalette);
  const store = useUiStore.getState;
  const projects = useProjects();
  const tasks = useTasks(null);
  const state = useRuntimeState();
  const scheduler = useSchedulerToggle();
  const refresh = useRefreshRuntime();
  const paused = state.data?.scheduler_paused ?? false;

  const run = (fn: () => void) => () => {
    close();
    fn();
  };

  const recentTasks = useMemo(
    () => [...(tasks.data ?? [])].sort((a, b) => (a.updated_at < b.updated_at ? 1 : -1)).slice(0, 60),
    [tasks.data]
  );

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? undefined : close())}>
      <DialogContent showClose={false} className="top-[20%] max-w-[640px] translate-y-0 overflow-hidden p-0" aria-label={m.palette.title}>
        <DialogTitle className="sr-only">{m.palette.title}</DialogTitle>
        <DialogDescription className="sr-only">{m.palette.placeholder}</DialogDescription>
        <Command className="bg-surface-raised" loop>
          <CommandInput placeholder={m.palette.placeholder} autoFocus />
          <CommandList className="max-h-[420px]">
            <CommandEmpty>{m.palette.empty}</CommandEmpty>
            {mode === "all" ? (
              <>
                <CommandGroup heading={m.palette.actions}>
                  <CommandItem onSelect={run(() => navigate(paths.home()))}>
                    <Plus /> {m.commandBar.newTask}
                    <CommandShortcut>{m.shortcuts.newTask}</CommandShortcut>
                  </CommandItem>
                  <CommandItem onSelect={run(() => store().setAddProjectOpen(true))}>
                    <FolderGit2 /> {m.palette.newProject}
                  </CommandItem>
                  <CommandItem onSelect={run(() => scheduler.mutate(!paused))}>
                    {paused ? <Play /> : <Pause />} {paused ? m.commandBar.resume : m.commandBar.pause}
                  </CommandItem>
                  <CommandItem onSelect={run(() => refresh.mutate())}>
                    <RefreshCw /> {m.commandBar.refresh}
                    <CommandShortcut>{m.shortcuts.refresh}</CommandShortcut>
                  </CommandItem>
                  <CommandItem
                    onSelect={run(() => {
                      const cur = store().themeMode;
                      store().setThemeMode(cur === "dark" ? "light" : cur === "light" ? "system" : "dark");
                    })}
                  >
                    <MoonStar /> {m.palette.toggleTheme}
                  </CommandItem>
                  <CommandItem onSelect={run(() => store().setDensity(store().density === "compact" ? "comfortable" : "compact"))}>
                    <Rows3 /> {m.palette.toggleDensity}
                  </CommandItem>
                  <CommandItem onSelect={run(() => store().setLocale(store().locale === "en" ? "sv" : "en"))}>
                    <Languages /> {m.palette.switchLanguage}
                  </CommandItem>
                  <CommandItem onSelect={run(() => store().setShortcutsOpen(true))}>
                    <Keyboard /> {m.palette.showShortcuts}
                    <CommandShortcut>{m.shortcuts.help}</CommandShortcut>
                  </CommandItem>
                </CommandGroup>
                <CommandSeparator />
                <CommandGroup heading={m.palette.navigate}>
                  {NAV_ITEMS.map((item) => {
                    const Icon = item.icon;
                    return (
                      <CommandItem key={item.key} value={`nav ${m.nav[item.key]}`} onSelect={run(() => navigate(item.path))}>
                        <Icon /> {m.nav[item.key]}
                      </CommandItem>
                    );
                  })}
                </CommandGroup>
                {recentTasks.length > 0 ? (
                  <>
                    <CommandSeparator />
                    <CommandGroup heading={m.palette.tasks}>
                      {recentTasks.map((task) => (
                        <CommandItem
                          key={task.id}
                          value={`task ${task.issue.identifier} ${task.issue.title}`}
                          onSelect={run(() => navigate(paths.run(task.id)))}
                        >
                          <SquareKanban />
                          <span className="font-mono text-xs">{task.issue.identifier}</span>
                          <span className="truncate">{task.issue.title}</span>
                          <CommandShortcut>{t.status[task.status]}</CommandShortcut>
                        </CommandItem>
                      ))}
                    </CommandGroup>
                  </>
                ) : null}
              </>
            ) : null}
            <CommandSeparator />
            <CommandGroup heading={m.palette.projects}>
              <CommandItem value="project __all" onSelect={run(() => store().setActiveProject(null))}>
                <FolderGit2 /> {m.project.all}
              </CommandItem>
              {(projects.data ?? []).map((p) => (
                <CommandItem key={p.id} value={`project ${p.name} ${p.repo_path}`} onSelect={run(() => store().setActiveProject(p.id))}>
                  <FolderGit2 /> {p.name}
                  <CommandShortcut className="max-w-48 truncate">{p.repo_path}</CommandShortcut>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </DialogContent>
    </Dialog>
  );
};
