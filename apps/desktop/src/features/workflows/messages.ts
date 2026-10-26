import { defineMessages } from "../../lib/i18n.js";

/** Strings for Workflow Studio (`/workflow`). */
export const workflowMessages = defineMessages({
  en: {
    title: "Workflow Studio",
    description: "Edit WORKFLOW.md — YAML front matter configures the runtime, the Markdown body is the agent prompt template.",
    editorLabel: "WORKFLOW.md editor",
    editorLoading: "Loading editor…",
    loading: "Loading workflow…",
    loadError: "Could not load the workflow",
    source: {
      label: "Source file",
      none: "No workflow file location configured",
      noneDetail: "The runtime has no active WORKFLOW.md or fallback path, so the document cannot be saved."
    },
    toolbar: {
      label: "Workflow actions",
      save: "Save",
      saving: "Saving…",
      saveHint: "Save to WORKFLOW.md (Ctrl+S)",
      saveDisabledClean: "No unsaved changes",
      saveDisabledInvalid: "Fix validation errors before saving",
      saveDisabledValidating: "Waiting for validation to finish",
      saveDisabledNoPath: "No workflow file location is configured",
      revert: "Revert",
      revertHint: "Discard unsaved changes and restore the saved document"
    },
    state: {
      dirty: "Unsaved changes",
      clean: "Saved",
      valid: "Valid",
      invalid: "{count, plural, one {# error} other {# errors}}",
      validating: "Validating…",
      previewFailed: "Validation unavailable"
    },
    tabs: {
      label: "Workflow inspection",
      validation: "Validation",
      config: "Effective config",
      prompt: "Prompt preview"
    },
    validation: {
      validTitle: "Workflow is valid",
      validBody: "The runtime parsed the front matter and rendered the prompt template without errors.",
      invalidTitle: "Workflow has errors",
      invalidBody: "The runtime rejects invalid workflows, so saving is disabled until every error is fixed.",
      validatingTitle: "Validating with the runtime…",
      requestFailed: "Could not validate the workflow",
      line: "Line {line}",
      lineColumn: "Line {line}, column {column}",
      noLocation: "No line information",
      goToLine: "Go to line {line}",
      rawDetails: "Runtime message",
      runtimeWarningTitle: "Runtime reports a workflow problem",
      summary: "Summary"
    },
    config: {
      emptyTitle: "No effective configuration",
      emptyBody: "Fix the validation errors to see the configuration the runtime would use.",
      redacted: "Tracker credentials are redacted by the runtime.",
      json: "Full configuration (JSON)",
      trackerKind: "Tracker",
      activeStates: "Active states",
      terminalStates: "Terminal states",
      requiredLabels: "Required labels",
      pollingInterval: "Polling interval",
      workspaceRoot: "Workspace root",
      maxAgents: "Max concurrent agents",
      maxTurns: "Max turns",
      maxBackoff: "Max retry backoff",
      codexCommand: "Codex command",
      approvalPolicy: "Approval policy",
      sandbox: "Thread sandbox",
      turnTimeout: "Turn timeout",
      hookTimeout: "Hook timeout",
      serverPort: "Server port",
      milliseconds: "{value} ms"
    },
    prompt: {
      emptyTitle: "No prompt preview",
      emptyBody: "The prompt template renders once the workflow is valid.",
      sampleNote: "Rendered by the runtime with a sample issue (attempt 1).",
      copy: "Copy rendered prompt"
    },
    external: {
      title: "WORKFLOW.md changed on disk",
      body: "The saved workflow changed while you have unsaved edits. Reload to discard your edits, or keep editing and save to overwrite it.",
      reload: "Reload from disk",
      keep: "Keep my edits"
    },
    confirm: {
      revertTitle: "Revert unsaved changes?",
      revertBody: "Your edits to WORKFLOW.md will be discarded and the saved document restored.",
      revertConfirm: "Revert",
      reloadTitle: "Reload the workflow from disk?",
      reloadBody: "Your unsaved edits will be discarded and replaced with the version saved on disk.",
      reloadConfirm: "Reload",
      leaveTitle: "Leave with unsaved changes?",
      leaveBody: "Your edits to WORKFLOW.md have not been saved and will be lost.",
      leaveConfirm: "Discard and leave",
      stay: "Stay"
    },
    toast: {
      saved: "Workflow saved",
      savedBody: "Saved to {path}. The runtime reloaded the configuration.",
      saveFailed: "Could not save the workflow"
    },
    errors: {
      unclosedFrontMatter: "The YAML front matter is missing its closing '---' line.",
      frontMatterNotMap: "The YAML front matter must be a map of keys to values.",
      unknownFilter: "Unknown Liquid filter '{name}'."
    }
  },
  sv: {
    title: "Arbetsflödesstudio",
    description: "Redigera WORKFLOW.md — YAML-huvudet konfigurerar körmiljön, Markdown-innehållet är agentens promptmall.",
    editorLabel: "Redigerare för WORKFLOW.md",
    editorLoading: "Läser in redigeraren…",
    loading: "Läser in arbetsflödet…",
    loadError: "Det gick inte att läsa in arbetsflödet",
    source: {
      label: "Källfil",
      none: "Ingen plats för arbetsflödesfilen är konfigurerad",
      noneDetail: "Körmiljön har ingen aktiv WORKFLOW.md eller reservsökväg, så dokumentet kan inte sparas."
    },
    toolbar: {
      label: "Åtgärder för arbetsflödet",
      save: "Spara",
      saving: "Sparar…",
      saveHint: "Spara till WORKFLOW.md (Ctrl+S)",
      saveDisabledClean: "Inga osparade ändringar",
      saveDisabledInvalid: "Åtgärda valideringsfelen innan du sparar",
      saveDisabledValidating: "Väntar på att valideringen ska bli klar",
      saveDisabledNoPath: "Ingen plats för arbetsflödesfilen är konfigurerad",
      revert: "Återställ",
      revertHint: "Ignorera osparade ändringar och återställ det sparade dokumentet"
    },
    state: {
      dirty: "Osparade ändringar",
      clean: "Sparat",
      valid: "Giltigt",
      invalid: "{count} fel",
      validating: "Validerar…",
      previewFailed: "Validering otillgänglig"
    },
    tabs: {
      label: "Granskning av arbetsflödet",
      validation: "Validering",
      config: "Effektiv konfiguration",
      prompt: "Förhandsvisning av prompt"
    },
    validation: {
      validTitle: "Arbetsflödet är giltigt",
      validBody: "Körmiljön tolkade YAML-huvudet och renderade promptmallen utan fel.",
      invalidTitle: "Arbetsflödet innehåller fel",
      invalidBody: "Körmiljön avvisar ogiltiga arbetsflöden, så det går inte att spara förrän alla fel är åtgärdade.",
      validatingTitle: "Validerar med körmiljön…",
      requestFailed: "Det gick inte att validera arbetsflödet",
      line: "Rad {line}",
      lineColumn: "Rad {line}, kolumn {column}",
      noLocation: "Ingen radinformation",
      goToLine: "Gå till rad {line}",
      rawDetails: "Meddelande från körmiljön",
      runtimeWarningTitle: "Körmiljön rapporterar ett problem med arbetsflödet",
      summary: "Sammanfattning"
    },
    config: {
      emptyTitle: "Ingen effektiv konfiguration",
      emptyBody: "Åtgärda valideringsfelen för att se den konfiguration som körmiljön skulle använda.",
      redacted: "Ärendesystemets inloggningsuppgifter maskeras av körmiljön.",
      json: "Fullständig konfiguration (JSON)",
      trackerKind: "Ärendesystem",
      activeStates: "Aktiva tillstånd",
      terminalStates: "Sluttillstånd",
      requiredLabels: "Obligatoriska etiketter",
      pollingInterval: "Avsökningsintervall",
      workspaceRoot: "Arbetsytans rot",
      maxAgents: "Max samtidiga agenter",
      maxTurns: "Max antal turer",
      maxBackoff: "Max väntetid mellan försök",
      codexCommand: "Codex-kommando",
      approvalPolicy: "Godkännandepolicy",
      sandbox: "Sandlåda för tråd",
      turnTimeout: "Tidsgräns per tur",
      hookTimeout: "Tidsgräns för krokar",
      serverPort: "Serverport",
      milliseconds: "{value} ms"
    },
    prompt: {
      emptyTitle: "Ingen förhandsvisning av prompten",
      emptyBody: "Promptmallen renderas när arbetsflödet är giltigt.",
      sampleNote: "Renderad av körmiljön med ett exempelärende (försök 1).",
      copy: "Kopiera renderad prompt"
    },
    external: {
      title: "WORKFLOW.md har ändrats på disken",
      body: "Det sparade arbetsflödet ändrades medan du har osparade ändringar. Läs in igen för att ignorera dina ändringar, eller fortsätt redigera och spara för att skriva över det.",
      reload: "Läs in från disken",
      keep: "Behåll mina ändringar"
    },
    confirm: {
      revertTitle: "Återställa osparade ändringar?",
      revertBody: "Dina ändringar i WORKFLOW.md ignoreras och det sparade dokumentet återställs.",
      revertConfirm: "Återställ",
      reloadTitle: "Läsa in arbetsflödet från disken igen?",
      reloadBody: "Dina osparade ändringar ignoreras och ersätts med versionen som är sparad på disken.",
      reloadConfirm: "Läs in",
      leaveTitle: "Lämna med osparade ändringar?",
      leaveBody: "Dina ändringar i WORKFLOW.md har inte sparats och kommer att gå förlorade.",
      leaveConfirm: "Ignorera och lämna",
      stay: "Stanna kvar"
    },
    toast: {
      saved: "Arbetsflödet har sparats",
      savedBody: "Sparat till {path}. Körmiljön läste in konfigurationen igen.",
      saveFailed: "Det gick inte att spara arbetsflödet"
    },
    errors: {
      unclosedFrontMatter: "YAML-huvudet saknar den avslutande raden '---'.",
      frontMatterNotMap: "YAML-huvudet måste vara en mappning av nycklar till värden.",
      unknownFilter: "Okänt Liquid-filter '{name}'."
    }
  }
});

export type WorkflowMessages = (typeof workflowMessages)["en"];
