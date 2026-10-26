import React, { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useBlocker } from "react-router-dom";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import {
  Button,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Tab,
  TabList,
  Toolbar,
  ToolbarButton,
  Tooltip
} from "@fluentui/react-components";
import { AlertCircle, CheckCircle2, RotateCcw, Save } from "lucide-react";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type ServiceConfig, type WorkflowValidation } from "@symphony/shared-types";
import {
  AiEmptyView,
  ConfirmDialog,
  ErrorState,
  KeyValueList,
  LoadingRows,
  PageHeader,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  SdkCodeBlock,
  SdkCopyButton,
  SdkMarkdown,
  useDefaultLayout,
  useNotify
} from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useSaveWorkflow, useWorkflow } from "../../lib/queries.js";
import { useClient } from "../../runtime/RuntimeProvider.js";
import { safeStateStorage } from "../../store/safeStorage.js";
import { workflowMessages } from "./messages.js";
import type { WorkflowEditorHandle } from "./WorkflowEditor.js";
import { diagnosticsFromValidation, type WorkflowDiagnostic } from "./workflowDiagnostics.js";

const WorkflowEditor = lazy(() => import("./WorkflowEditor.js"));

type Messages = (typeof workflowMessages)["en"];
type InspectTab = "validation" | "config" | "prompt";

const VALIDATE_DEBOUNCE_MS = 400;

function useDebounced<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return debounced;
}

/** Runtime-side validation of the editor text (always 200; `valid` says whether it would load). */
function useWorkflowPreview(raw: string | null) {
  const client = useClient();
  return useQuery({
    queryKey: ["workflow", "preview", raw],
    queryFn: ({ signal }) => client.previewWorkflow({ raw_markdown: raw ?? "" }, signal),
    enabled: raw !== null,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 30_000
  });
}

function diagnosticText(m: Messages, d: WorkflowDiagnostic): string {
  if (!d.known) return d.message;
  if (d.known.key === "unknownFilter") return formatMessage(m.errors.unknownFilter, { name: d.known.name });
  return m.errors[d.known.key];
}

const ValidationTab: React.FC<{
  m: Messages;
  validating: boolean;
  validation: WorkflowValidation | undefined;
  requestError: string | null;
  diagnostics: WorkflowDiagnostic[];
  onGoTo: (d: WorkflowDiagnostic) => void;
}> = ({ m, validating, validation, requestError, diagnostics, onGoTo }) => {
  if (requestError) return <ErrorState className="m-3" title={m.validation.requestFailed} message={requestError} />;
  if (!validation) return <LoadingRows className="p-3" rows={3} label={m.validation.validatingTitle} />;
  if (validation.valid) {
    return (
      <MessageBar intent="success" className="m-3">
        <MessageBarBody>
          <MessageBarTitle>{m.validation.validTitle}</MessageBarTitle>
          {m.validation.validBody}
        </MessageBarBody>
      </MessageBar>
    );
  }
  return (
    <div className="space-y-2 p-3" aria-busy={validating}>
      <MessageBar intent="error">
        <MessageBarBody>
          <MessageBarTitle>{m.validation.invalidTitle}</MessageBarTitle>
          {m.validation.invalidBody}
        </MessageBarBody>
      </MessageBar>
      <ul className="space-y-2">
        {diagnostics.map((d, index) => (
          <li key={index} className="space-y-1 rounded-md border border-border bg-surface p-2 text-sm">
            <div className="flex items-start gap-2">
              <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
              <p className="min-w-0 flex-1 text-foreground">{diagnosticText(m, d)}</p>
              {d.line !== null ? (
                <Button size="small" appearance="subtle" onClick={() => onGoTo(d)}>
                  {formatMessage(m.validation.goToLine, { line: d.line })}
                </Button>
              ) : null}
            </div>
            <p className="pl-6 text-xs text-muted-foreground">
              {d.line === null
                ? m.validation.noLocation
                : d.column !== null
                  ? formatMessage(m.validation.lineColumn, { line: d.line, column: d.column })
                  : formatMessage(m.validation.line, { line: d.line })}
              {d.code ? <span className="ml-2 font-mono">{d.code}</span> : null}
            </p>
            {d.raw.trim() !== d.message ? (
              <details className="pl-6 text-xs">
                <summary className="cursor-pointer text-muted-foreground">{m.validation.rawDetails}</summary>
                <pre tabIndex={0} className="mt-1 overflow-auto whitespace-pre-wrap rounded bg-code p-2 font-mono text-foreground">{d.raw}</pre>
              </details>
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  );
};

const ConfigTab: React.FC<{ m: Messages; config: ServiceConfig | null | undefined }> = ({ m, config }) => {
  if (!config) return <AiEmptyView className="m-3" title={m.config.emptyTitle} description={m.config.emptyBody} />;
  const ms = (value: number) => formatMessage(m.config.milliseconds, { value });
  const list = (values: string[]) => (values.length ? values.join(", ") : "—");
  return (
    <div className="space-y-3 p-3">
      <p className="text-xs text-muted-foreground">{m.config.redacted}</p>
      <KeyValueList
        items={[
          { label: m.config.trackerKind, value: config.tracker_kind || "—" },
          { label: m.config.activeStates, value: list(config.active_states) },
          { label: m.config.terminalStates, value: list(config.terminal_states) },
          { label: m.config.requiredLabels, value: list(config.required_labels) },
          { label: m.config.pollingInterval, value: ms(config.polling_interval_ms) },
          { label: m.config.workspaceRoot, value: config.workspace_root, mono: true },
          { label: m.config.maxAgents, value: config.agent.max_concurrent_agents },
          { label: m.config.maxTurns, value: config.agent.max_turns },
          { label: m.config.maxBackoff, value: ms(config.agent.max_retry_backoff_ms) },
          { label: m.config.codexCommand, value: config.codex.command, mono: true },
          { label: m.config.approvalPolicy, value: JSON.stringify(config.codex.approval_policy), mono: true },
          { label: m.config.sandbox, value: config.codex.thread_sandbox, mono: true },
          { label: m.config.turnTimeout, value: ms(config.codex.turn_timeout_ms) },
          { label: m.config.hookTimeout, value: ms(config.hooks.timeout_ms) },
          { label: m.config.serverPort, value: config.server_port ?? "—" }
        ]}
      />
      <details>
        <summary className="cursor-pointer text-xs text-muted-foreground">{m.config.json}</summary>
        <SdkCodeBlock language="json">{JSON.stringify(config, null, 2)}</SdkCodeBlock>
      </details>
    </div>
  );
};

const PromptTab: React.FC<{ m: Messages; prompt: string | null | undefined }> = ({ m, prompt }) => {
  if (!prompt) return <AiEmptyView className="m-3" title={m.prompt.emptyTitle} description={m.prompt.emptyBody} />;
  return (
    <div className="space-y-2 p-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">{m.prompt.sampleNote}</p>
        <SdkCopyButton copyValue={prompt} aria-label={m.prompt.copy} size="sm" variant="ghost" color="secondary" />
      </div>
      <div className="rounded-md border border-border bg-surface p-3 text-sm">
        <SdkMarkdown>{prompt}</SdkMarkdown>
      </div>
    </div>
  );
};

/** Workflow Studio: edit WORKFLOW.md with live runtime validation, effective config and prompt preview. */
export default function WorkflowStudioPage() {
  const m = useMessages(workflowMessages);
  const notify = useNotify();
  const doc = useWorkflow();
  const save = useSaveWorkflow();
  const editorRef = useRef<WorkflowEditorHandle>(null);
  const layout = useDefaultLayout({ id: "symphony-workflow", storage: safeStateStorage });

  const [baseline, setBaseline] = useState<string | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [diskVersion, setDiskVersion] = useState<string | null>(null);
  const [tab, setTab] = useState<InspectTab>("validation");
  const [confirm, setConfirm] = useState<"revert" | "reload" | null>(null);

  const dirty = draft !== null && baseline !== null && draft !== baseline;

  // Adopt the server document on load and whenever it changes on disk while there are no edits.
  const serverRaw = doc.data?.raw_markdown;
  useEffect(() => {
    if (serverRaw === undefined) return;
    if (baseline === null || draft === baseline) {
      setBaseline(serverRaw);
      setDraft(serverRaw);
      setDiskVersion(null);
    } else if (serverRaw !== baseline) {
      setDiskVersion(serverRaw);
    }
    // Only react to new server content.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverRaw]);

  const validated = useDebounced(draft, VALIDATE_DEBOUNCE_MS);
  const preview = useWorkflowPreview(validated);
  const validation = preview.data;
  const validating = draft !== validated || preview.isFetching;
  const diagnostics = useMemo(
    () => (validated !== null ? diagnosticsFromValidation(validated, validation) : []),
    [validated, validation]
  );
  const path = doc.data?.path ?? validation?.path ?? null;

  const saveBlockedReason = !path
    ? m.toolbar.saveDisabledNoPath
    : !dirty
      ? m.toolbar.saveDisabledClean
      : validating
        ? m.toolbar.saveDisabledValidating
        : !validation?.valid
          ? m.toolbar.saveDisabledInvalid
          : null;

  // Synchronous guard: two clicks in the same frame must not submit twice.
  const saving = useRef(false);
  const doSave = useCallback(() => {
    if (saveBlockedReason || save.isPending || saving.current || draft === null) return;
    saving.current = true;
    const content = draft;
    save.mutate(content, {
      onSettled: () => {
        saving.current = false;
      },
      onSuccess: (result) => {
        setBaseline(content);
        setDiskVersion(null);
        notify("success", m.toast.saved, formatMessage(m.toast.savedBody, { path: result.path ?? path ?? "" }));
      },
      onError: (error) => notify("error", m.toast.saveFailed, errorMessage(error))
    });
  }, [saveBlockedReason, save, draft, notify, m, path]);

  // Ctrl+S anywhere on the page (the editor has its own binding when focused).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        doSave();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [doSave]);

  const blocker = useBlocker(({ currentLocation, nextLocation }) => dirty && currentLocation.pathname !== nextLocation.pathname);

  const goTo = (d: WorkflowDiagnostic) => {
    if (d.line !== null) editorRef.current?.focusLine(d.line, d.column);
  };

  if (doc.isLoading) return <LoadingRows className="p-4" rows={8} label={m.loading} />;
  if (doc.isError) return <ErrorState className="m-4" title={m.loadError} message={errorMessage(doc.error)} onRetry={() => void doc.refetch()} />;

  const status = validating ? (
    <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
      <Spinner size="extra-tiny" /> {m.state.validating}
    </span>
  ) : preview.isError ? (
    <span className="text-xs text-warning">{m.state.previewFailed}</span>
  ) : validation?.valid ? (
    <span className="flex items-center gap-1 text-xs text-success">
      <CheckCircle2 className="size-3.5" aria-hidden /> {m.state.valid}
    </span>
  ) : (
    <span className="flex items-center gap-1 text-xs text-destructive">
      <AlertCircle className="size-3.5" aria-hidden /> {formatMessage(m.state.invalid, { count: Math.max(diagnostics.length, 1) })}
    </span>
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title={m.title}
        description={m.description}
        meta={
          <span className="flex items-center gap-3" aria-live="polite">
            <span className={dirty ? "text-xs text-warning" : "text-xs text-muted-foreground"}>{dirty ? m.state.dirty : m.state.clean}</span>
            {status}
          </span>
        }
        actions={
          <Toolbar aria-label={m.toolbar.label} size="small">
            <Tooltip content={saveBlockedReason ?? m.toolbar.saveHint} relationship="description">
              <ToolbarButton
                appearance="primary"
                icon={save.isPending ? <Spinner size="extra-tiny" /> : <Save className="size-4" />}
                disabledFocusable={Boolean(saveBlockedReason) || save.isPending}
                onClick={doSave}
              >
                {save.isPending ? m.toolbar.saving : m.toolbar.save}
              </ToolbarButton>
            </Tooltip>
            <Tooltip content={m.toolbar.revertHint} relationship="description">
              <ToolbarButton icon={<RotateCcw className="size-4" />} disabled={!dirty || save.isPending} onClick={() => setConfirm("revert")}>
                {m.toolbar.revert}
              </ToolbarButton>
            </Tooltip>
          </Toolbar>
        }
      />
      <div className="flex shrink-0 items-center gap-2 border-b border-border bg-surface px-4 py-1 text-xs">
        <span className="text-muted-foreground">{m.source.label}:</span>
        {path ? (
          <code className="truncate font-mono text-foreground" title={path}>
            {path}
          </code>
        ) : (
          <span className="text-warning" title={m.source.noneDetail}>
            {m.source.none}
          </span>
        )}
      </div>
      {diskVersion !== null ? (
        <MessageBar intent="warning" className="mx-4 mt-2 shrink-0">
          <MessageBarBody>
            <MessageBarTitle>{m.external.title}</MessageBarTitle>
            {m.external.body}
          </MessageBarBody>
          <MessageBarActions>
            <Button size="small" onClick={() => setConfirm("reload")}>
              {m.external.reload}
            </Button>
            <Button size="small" appearance="subtle" onClick={() => setDiskVersion(null)}>
              {m.external.keep}
            </Button>
          </MessageBarActions>
        </MessageBar>
      ) : null}
      {doc.data && !doc.data.valid && doc.data.error && !dirty ? (
        <MessageBar intent="error" className="mx-4 mt-2 shrink-0">
          <MessageBarBody>
            <MessageBarTitle>{m.validation.runtimeWarningTitle}</MessageBarTitle>
            {doc.data.error}
          </MessageBarBody>
        </MessageBar>
      ) : null}
      <div className="min-h-0 flex-1 p-2">
        <ResizablePanelGroup orientation="horizontal" id="symphony-workflow" {...layout}>
          <ResizablePanel id="editor" defaultSize="58" minSize="30">
            <div className="h-full overflow-hidden rounded-md border border-border">
              <Suspense fallback={<LoadingRows className="p-3" rows={10} label={m.editorLoading} />}>
                {draft !== null ? (
                  <WorkflowEditor ref={editorRef} value={draft} onChange={setDraft} onSave={doSave} diagnostics={diagnostics} label={m.editorLabel} />
                ) : null}
              </Suspense>
            </div>
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel id="inspect" defaultSize="42" minSize="24">
            <section className="flex h-full min-h-0 flex-col overflow-hidden rounded-md border border-border bg-surface" aria-label={m.tabs.label}>
              <TabList size="small" selectedValue={tab} onTabSelect={(_, d) => setTab(d.value as InspectTab)} aria-label={m.tabs.label} className="shrink-0 border-b border-border">
                <Tab value="validation">{m.tabs.validation}</Tab>
                <Tab value="config">{m.tabs.config}</Tab>
                <Tab value="prompt">{m.tabs.prompt}</Tab>
              </TabList>
              <div className="min-h-0 flex-1 overflow-auto">
                {tab === "validation" ? (
                  <ValidationTab
                    m={m}
                    validating={validating}
                    validation={validation}
                    requestError={preview.isError ? errorMessage(preview.error) : null}
                    diagnostics={diagnostics}
                    onGoTo={goTo}
                  />
                ) : null}
                {tab === "config" ? <ConfigTab m={m} config={validation?.config} /> : null}
                {tab === "prompt" ? <PromptTab m={m} prompt={validation?.rendered_prompt} /> : null}
              </div>
            </section>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>

      <ConfirmDialog
        open={confirm === "revert"}
        title={m.confirm.revertTitle}
        description={m.confirm.revertBody}
        confirmLabel={m.confirm.revertConfirm}
        destructive
        onOpenChange={(open) => !open && setConfirm(null)}
        onConfirm={() => {
          setDraft(baseline);
          setConfirm(null);
        }}
      />
      <ConfirmDialog
        open={confirm === "reload"}
        title={m.confirm.reloadTitle}
        description={m.confirm.reloadBody}
        confirmLabel={m.confirm.reloadConfirm}
        destructive
        onOpenChange={(open) => !open && setConfirm(null)}
        onConfirm={() => {
          if (diskVersion !== null) {
            setBaseline(diskVersion);
            setDraft(diskVersion);
          }
          setDiskVersion(null);
          setConfirm(null);
        }}
      />
      <ConfirmDialog
        open={blocker.state === "blocked"}
        title={m.confirm.leaveTitle}
        description={m.confirm.leaveBody}
        confirmLabel={m.confirm.leaveConfirm}
        cancelLabel={m.confirm.stay}
        destructive
        onOpenChange={(open) => {
          if (!open && blocker.state === "blocked") blocker.reset();
        }}
        onConfirm={() => {
          if (blocker.state === "blocked") blocker.proceed();
        }}
      />
    </div>
  );
}
