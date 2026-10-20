import React, { Suspense, lazy } from "react";
import { HashRouter, Navigate, Route, Routes, useLocation } from "react-router-dom";
import { LoadingRows } from "@symphony/ui";
import { DesktopShell } from "../layouts/DesktopShell.js";
import { ErrorBoundary } from "../components/ErrorBoundary.js";
import { useMessages } from "../lib/i18n.js";
import { useUiStore } from "../store/uiStore.js";
import { shellMessages } from "./messages.js";

// Each feature view is code-split; heavy views (Workflow Studio's editor, execution timeline)
// load only when opened.
const OverviewPage = lazy(() => import("../features/overview/OverviewPage.js"));
const ProjectsPage = lazy(() => import("../features/projects/ProjectsPage.js"));
const TasksPage = lazy(() => import("../features/tasks/TasksPage.js"));
const AgentsPage = lazy(() => import("../features/agents/AgentsPage.js"));
const ExecutionPage = lazy(() => import("../features/execution/ExecutionPage.js"));
const WorktreesPage = lazy(() => import("../features/worktrees/WorktreesPage.js"));
const ReviewsPage = lazy(() => import("../features/reviews/ReviewsPage.js"));
const WorkflowStudioPage = lazy(() => import("../features/workflows/WorkflowStudioPage.js"));
const ProvidersPage = lazy(() => import("../features/providers/ProvidersPage.js"));
const SettingsPage = lazy(() => import("../features/settings/SettingsPage.js"));

const RouteFrame: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const location = useLocation();
  const m = useMessages(shellMessages);
  return (
    <ErrorBoundary title={m.errors.boundaryTitle} detail={m.errors.boundaryDetail} retryLabel={m.errors.reloadView} resetKey={location.pathname}>
      <Suspense fallback={<LoadingRows className="p-4" rows={8} />}>{children}</Suspense>
    </ErrorBoundary>
  );
};

const RestoreLastRoute: React.FC = () => {
  const lastRoute = useUiStore((s) => s.lastRoute);
  const safe = lastRoute.startsWith("/") && lastRoute !== "/" ? lastRoute : "/overview";
  return <Navigate to={safe} replace />;
};

const frame = (el: React.ReactNode) => <RouteFrame>{el}</RouteFrame>;

export const AppRoutes: React.FC = () => (
  <Routes>
    <Route element={<DesktopShell />}>
      <Route index element={<RestoreLastRoute />} />
      <Route path="overview" element={frame(<OverviewPage />)} />
      <Route path="projects" element={frame(<ProjectsPage />)} />
      <Route path="projects/:projectId" element={frame(<ProjectsPage />)} />
      <Route path="tasks" element={frame(<TasksPage />)} />
      <Route path="runs" element={frame(<AgentsPage />)} />
      <Route path="runs/:taskId" element={frame(<ExecutionPage />)} />
      <Route path="worktrees" element={frame(<WorktreesPage />)} />
      <Route path="reviews" element={frame(<ReviewsPage />)} />
      <Route path="reviews/:reviewId" element={frame(<ReviewsPage />)} />
      <Route path="workflow" element={frame(<WorkflowStudioPage />)} />
      <Route path="providers" element={frame(<ProvidersPage />)} />
      <Route path="settings" element={frame(<SettingsPage />)} />
      <Route path="*" element={<Navigate to="/overview" replace />} />
    </Route>
  </Routes>
);

/** Hash routing works identically under `tauri://localhost`, the dev server and E2E tests. */
export const AppRouter: React.FC = () => (
  <HashRouter>
    <AppRoutes />
  </HashRouter>
);
