import React, { createContext, useCallback, useContext, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import type { GraphLevel } from "@symphony/shared-types";

export type KnowledgeTab = "overview" | "graph" | "commits" | "worktrees" | "insights";
export const KNOWLEDGE_TABS: ReadonlyArray<KnowledgeTab> = ["overview", "graph", "commits", "worktrees", "insights"];

/** The commit sub-views of the detail panel. */
export type CommitPanelTab = "diff" | "impact" | "review" | "tests";

export interface KnowledgeSelection {
  projectId: string | null;
  tab: KnowledgeTab;
  /** Selected commit (full id). It drives the commit panel, the graph highlight and the strip. */
  sha: string | null;
  /** Selected node of the code graph. */
  nodeId: string | null;
  /** Worktree the commit list is narrowed to. */
  worktreeId: string | null;
  /** Detail level of the graph, and the module the file level is scoped to. */
  level: GraphLevel;
  module: string | null;
  setTab(tab: KnowledgeTab): void;
  setLevel(level: GraphLevel): void;
  /** Show a module's files in the graph. */
  openModule(module: string): void;
  /** Show the neighbourhood of a node in the graph. */
  focusNode(nodeId: string): void;
  /** Select a commit; `tab` also switches the page (for links such as "Show in graph"). */
  selectCommit(sha: string | null, tab?: KnowledgeTab): void;
  selectNode(nodeId: string | null, tab?: KnowledgeTab): void;
  selectWorktree(worktreeId: string | null, tab?: KnowledgeTab): void;
}

const Context = createContext<KnowledgeSelection | null>(null);

export function useKnowledgeSelection(): KnowledgeSelection {
  const value = useContext(Context);
  if (!value) throw new Error("useKnowledgeSelection must be used inside <KnowledgeSelectionProvider>");
  return value;
}

/**
 * The page's selection lives in the URL (`?tab=&commit=&node=&worktree=`), so a view can be
 * reloaded, restored after a restart and linked to from elsewhere in the app.
 */
export const KnowledgeSelectionProvider: React.FC<{ projectId: string | null; children: React.ReactNode }> = ({ projectId, children }) => {
  const [params, setParams] = useSearchParams();
  const rawTab = params.get("tab");
  const tab: KnowledgeTab = KNOWLEDGE_TABS.includes(rawTab as KnowledgeTab) ? (rawTab as KnowledgeTab) : "overview";
  const sha = params.get("commit");
  const nodeId = params.get("node");
  const worktreeId = params.get("worktree");
  const rawLevel = params.get("level");
  const level: GraphLevel = rawLevel === "files" || rawLevel === "symbols" ? rawLevel : "modules";
  const module = params.get("module");

  const update = useCallback(
    (changes: Record<string, string | null>) => {
      setParams(
        (previous) => {
          const next = new URLSearchParams(previous);
          for (const [key, value] of Object.entries(changes)) {
            if (value === null || value === "") next.delete(key);
            else next.set(key, value);
          }
          return next;
        },
        { replace: true }
      );
    },
    [setParams]
  );

  const value = useMemo<KnowledgeSelection>(
    () => ({
      projectId,
      tab,
      sha,
      nodeId,
      worktreeId,
      level,
      module,
      setTab: (next) => update({ tab: next === "overview" ? null : next }),
      setLevel: (next) => update({ level: next === "modules" ? null : next, module: next === "files" ? module : null }),
      openModule: (next) => update({ tab: "graph", level: "files", module: next, node: null }),
      focusNode: (next) => update({ tab: "graph", level: "symbols", node: next }),
      selectCommit: (next, nextTab) => update({ commit: next, ...(nextTab ? { tab: nextTab } : {}) }),
      selectNode: (next, nextTab) => update({ node: next, ...(nextTab ? { tab: nextTab } : {}) }),
      selectWorktree: (next, nextTab) => update({ worktree: next, ...(nextTab ? { tab: nextTab } : {}) })
    }),
    [projectId, tab, sha, nodeId, worktreeId, level, module, update]
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
};
