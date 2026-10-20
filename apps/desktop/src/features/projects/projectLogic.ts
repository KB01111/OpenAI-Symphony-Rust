/** Pure project-form and list logic (unit tested; no React). */
import type { CreateProjectRequest, Project, ProjectUpdate, ProviderKind, Task, TaskStatus } from "@symphony/shared-types";

export const TRACKER_KINDS = ["manual", "github", "linear"] as const;
export type KnownTrackerKind = (typeof TRACKER_KINDS)[number];

export interface ProjectFormState {
  name: string;
  defaultBranch: string;
  provider: ProviderKind;
  model: string;
  trackerKind: string;
  githubRepo: string;
  linearSlug: string;
  requiredLabels: string;
  autoDispatch: boolean;
  maxAgents: number;
  validationCommands: string;
}

export type ProjectFormErrors = Partial<Record<"name" | "defaultBranch" | "maxAgents" | "githubRepo", true>>;

export function projectToForm(p: Project): ProjectFormState {
  return {
    name: p.name,
    defaultBranch: p.default_branch,
    provider: p.preferred_provider,
    model: p.preferred_model ?? "",
    trackerKind: p.tracker_kind,
    githubRepo: p.github_repo ?? "",
    linearSlug: p.linear_project_slug ?? "",
    requiredLabels: p.required_labels.join(", "),
    autoDispatch: p.auto_dispatch,
    maxAgents: p.max_concurrent_agents,
    validationCommands: p.validation_commands.join("\n")
  };
}

/** Comma separated labels, trimmed and de-duplicated (order preserved). */
export function parseLabels(input: string): string[] {
  const seen = new Set<string>();
  for (const part of input.split(",")) {
    const label = part.trim();
    if (label) seen.add(label);
  }
  return [...seen];
}

/** One command per non-empty line. */
export function parseLines(input: string): string[] {
  return input
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);
}

const GITHUB_REPO = /^[\w.-]+\/[\w.-]+$/;

export function validateProjectForm(form: ProjectFormState): ProjectFormErrors {
  const errors: ProjectFormErrors = {};
  if (!form.name.trim()) errors.name = true;
  if (!form.defaultBranch.trim()) errors.defaultBranch = true;
  if (!Number.isInteger(form.maxAgents) || form.maxAgents < 1 || form.maxAgents > 32) errors.maxAgents = true;
  if (form.trackerKind === "github" && form.githubRepo.trim() && !GITHUB_REPO.test(form.githubRepo.trim())) errors.githubRepo = true;
  return errors;
}

export function hasErrors(errors: ProjectFormErrors): boolean {
  return Object.keys(errors).length > 0;
}

function sameList(a: ReadonlyArray<string>, b: ReadonlyArray<string>): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

/** Minimal PATCH body: only fields that differ from the saved project. */
export function formToUpdate(project: Project, form: ProjectFormState): ProjectUpdate {
  const update: ProjectUpdate = {};
  if (form.name.trim() !== project.name) update.name = form.name.trim();
  if (form.defaultBranch.trim() !== project.default_branch) update.default_branch = form.defaultBranch.trim();
  if (form.provider !== project.preferred_provider) update.preferred_provider = form.provider;
  if (form.model.trim() !== (project.preferred_model ?? "")) update.preferred_model = form.model.trim();
  if (form.trackerKind !== project.tracker_kind) update.tracker_kind = form.trackerKind;
  if (form.githubRepo.trim() !== (project.github_repo ?? "")) update.github_repo = form.githubRepo.trim();
  if (form.linearSlug.trim() !== (project.linear_project_slug ?? "")) update.linear_project_slug = form.linearSlug.trim();
  const labels = parseLabels(form.requiredLabels);
  if (!sameList(labels, project.required_labels)) update.required_labels = labels;
  if (form.autoDispatch !== project.auto_dispatch) update.auto_dispatch = form.autoDispatch;
  if (form.maxAgents !== project.max_concurrent_agents) update.max_concurrent_agents = form.maxAgents;
  const commands = parseLines(form.validationCommands);
  if (!sameList(commands, project.validation_commands)) update.validation_commands = commands;
  return update;
}

export function isDirty(project: Project, form: ProjectFormState): boolean {
  return Object.keys(formToUpdate(project, form)).length > 0;
}

/** Defaults for the add-repository wizard (mirrors the runtime's create defaults). */
export function emptyProjectForm(provider: ProviderKind, name = "", branch = ""): ProjectFormState {
  return {
    name,
    defaultBranch: branch,
    provider,
    model: "",
    trackerKind: "manual",
    githubRepo: "",
    linearSlug: "",
    requiredLabels: "symphony",
    autoDispatch: false,
    maxAgents: 2,
    validationCommands: ""
  };
}

export function formToCreate(repoPath: string, workspaceRoot: string, form: ProjectFormState): CreateProjectRequest {
  return {
    name: form.name.trim(),
    repo_path: repoPath,
    default_branch: form.defaultBranch.trim() || null,
    workspace_root: workspaceRoot.trim() || null,
    preferred_provider: form.provider,
    preferred_model: form.model.trim() || null,
    tracker_kind: form.trackerKind,
    github_repo: form.trackerKind === "github" ? form.githubRepo.trim() || null : null,
    linear_project_slug: form.trackerKind === "linear" ? form.linearSlug.trim() || null : null,
    required_labels: parseLabels(form.requiredLabels),
    auto_dispatch: form.autoDispatch,
    max_concurrent_agents: form.maxAgents,
    validation_commands: parseLines(form.validationCommands)
  };
}

export interface ProjectTaskCounts {
  total: number;
  running: number;
  review: number;
  failed: number;
  byStatus: Partial<Record<TaskStatus, number>>;
}

export function countTasksByProject(tasks: ReadonlyArray<Task>): Map<string, ProjectTaskCounts> {
  const map = new Map<string, ProjectTaskCounts>();
  for (const task of tasks) {
    let counts = map.get(task.project_id);
    if (!counts) {
      counts = { total: 0, running: 0, review: 0, failed: 0, byStatus: {} };
      map.set(task.project_id, counts);
    }
    counts.total += 1;
    counts.byStatus[task.status] = (counts.byStatus[task.status] ?? 0) + 1;
    if (task.status === "running") counts.running += 1;
    if (task.status === "human_review") counts.review += 1;
    if (task.status === "failed") counts.failed += 1;
  }
  return map;
}

export function filterProjects(projects: ReadonlyArray<Project>, search: string, tracker: string | null): Project[] {
  const q = search.trim().toLowerCase();
  return projects.filter((p) => {
    if (tracker && p.tracker_kind !== tracker) return false;
    if (!q) return true;
    return [p.name, p.repo_path, p.default_branch, p.github_repo ?? "", p.linear_project_slug ?? "", p.tracker_kind].some((v) =>
      v.toLowerCase().includes(q)
    );
  });
}
