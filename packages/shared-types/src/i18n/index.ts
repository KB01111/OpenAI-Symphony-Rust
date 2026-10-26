import type {
  ApprovalDecision,
  ApprovalStatus,
  ProviderAuthState,
  ProviderKind,
  RunAttemptPhase,
  TaskStatus
} from "../generated/openapi/types.gen.js";
import type { TransitionRejection } from "../status.js";

export type Locale = "en" | "sv";
export const SUPPORTED_LOCALES: ReadonlyArray<Locale> = ["en", "sv"];

/**
 * Typed message catalogue helper. Feature modules declare their own catalogues with
 * `defineMessages({ en: {...}, sv: {...} })`; the Swedish catalogue must have exactly the same
 * shape as the English one or the build fails.
 */
export function defineMessages<T>(catalogue: { en: T; sv: NoInfer<T> }): Record<Locale, T> {
  return catalogue;
}

const PLURAL = /\{(\w+),\s*plural,((?:\s*(?:=\d+|zero|one|two|few|many|other)\s*\{[^{}]*\})+)\s*\}/g;
const PLURAL_CASE = /(=\d+|zero|one|two|few|many|other)\s*\{([^{}]*)\}/g;

/**
 * Fills `{name}` placeholders. Counts can choose a form with the ICU plural subset
 * `{count, plural, one {# task} other {# tasks}}` (`=N` exact matches, `#` is the number); English
 * and Swedish only distinguish `one` (exactly 1) from `other`. Unknown placeholders are left untouched.
 */
export function formatMessage(
  template: string,
  values: Record<string, string | number | null | undefined>
): string {
  const plural = template.replace(PLURAL, (match, key: string, cases: string) => {
    const value = values[key];
    if (value === undefined || value === null) return match;
    const n = Number(value);
    const forms = new Map<string, string>();
    for (const [, selector, text] of cases.matchAll(PLURAL_CASE)) forms.set(selector!, text!);
    const chosen = forms.get(`=${n}`) ?? (n === 1 ? forms.get("one") : undefined) ?? forms.get("other") ?? match;
    return chosen.replace(/#/g, String(value));
  });
  return plural.replace(/\{(\w+)\}/g, (match, key: string) => {
    const value = values[key];
    return value === undefined || value === null ? match : String(value);
  });
}

const en = {
  appName: "Symphony",
  common: {
    save: "Save",
    saving: "Saving…",
    cancel: "Cancel",
    close: "Close",
    confirm: "Confirm",
    delete: "Delete",
    retry: "Retry",
    refresh: "Refresh",
    reload: "Reload",
    copy: "Copy",
    copied: "Copied",
    search: "Search",
    filter: "Filter",
    clearFilters: "Clear filters",
    loading: "Loading…",
    none: "None",
    unknown: "Unknown",
    all: "All",
    yes: "Yes",
    no: "No",
    open: "Open",
    details: "Details",
    back: "Back",
    next: "Next",
    create: "Create",
    edit: "Edit",
    apply: "Apply",
    discard: "Discard",
    dismiss: "Dismiss",
    showMore: "Show more",
    showLess: "Show less",
    tokens: "tokens",
    justNow: "just now",
    secondsAgo: "{count}s ago",
    minutesAgo: "{count}m ago",
    hoursAgo: "{count}h ago",
    daysAgo: "{count}d ago",
    durationSeconds: "{count}s",
    durationMinutes: "{minutes}m {seconds}s",
    durationHours: "{hours}h {minutes}m"
  },
  status: {
    backlog: "Backlog",
    ready: "Ready",
    queued: "Queued",
    running: "Running",
    blocked: "Blocked",
    human_review: "Human review",
    completed: "Completed",
    failed: "Failed",
    cancelled: "Cancelled"
  } satisfies Record<TaskStatus, string>,
  phase: {
    preparing_workspace: "Preparing workspace",
    building_prompt: "Building prompt",
    launching_agent_process: "Launching agent",
    initializing_session: "Initializing session",
    streaming_turn: "Streaming turn",
    finishing: "Finishing",
    succeeded: "Succeeded",
    failed: "Failed",
    timed_out: "Timed out",
    stalled: "Stalled",
    canceled_by_reconciliation: "Cancelled"
  } satisfies Record<RunAttemptPhase, string>,
  provider: {
    codex: "OpenAI Codex",
    claude_code: "Claude Code",
    opencode: "OpenCode",
    simulated: "Simulated agent"
  } satisfies Record<ProviderKind, string>,
  authState: {
    authenticated: "Authenticated",
    unauthenticated: "Not signed in",
    unknown: "Unverified",
    not_required: "Not required"
  } satisfies Record<ProviderAuthState, string>,
  approvalDecision: {
    accept: "Approve once",
    accept_for_session: "Approve for session",
    decline: "Decline",
    cancel: "Abort turn"
  } satisfies Record<ApprovalDecision, string>,
  approvalStatus: {
    pending: "Pending",
    approved: "Approved",
    denied: "Denied",
    cancelled: "Cancelled"
  } satisfies Record<ApprovalStatus, string>,
  transitionRejection: {
    same_status: "The task is already in this column.",
    use_dispatch: "Tasks start running only through dispatch. Use Start instead.",
    active_session: "An agent session is active. Cancel it before changing status.",
    not_allowed: "The runtime state machine does not allow moving from {from} to {to}."
  } satisfies Record<TransitionRejection, string>,
  connection: {
    connecting: "Connecting to runtime…",
    connected: "Runtime connected",
    reconnecting: "Reconnecting…",
    disconnected: "Runtime disconnected",
    unauthorized: "Authentication required",
    paused: "Dispatching paused",
    live: "Live",
    polling: "Polling",
    stale: "Data may be stale"
  },
  errors: {
    generic: "Something went wrong.",
    network: "The runtime could not be reached.",
    unauthorized: "The runtime rejected the access token.",
    notFound: "The requested item no longer exists.",
    conflict: "The request conflicts with the current state.",
    validation: "The request was rejected: {message}"
  },
  mobile: {
    title: "Symphony Remote",
    connection: "Connection",
    hostUrl: "Runtime URL",
    accessToken: "Access token",
    connect: "Connect",
    disconnect: "Forget connection",
    tokenStoredNotice: "The token is kept for this session only unless you choose to remember it.",
    rememberToken: "Remember on this device",
    activeAgents: "Active agents",
    pendingApprovals: "Pending approvals",
    recentTasks: "Recent tasks",
    newTask: "New task",
    project: "Project",
    taskTitle: "Title",
    taskDescription: "Description",
    dispatchNow: "Dispatch immediately",
    createTask: "Create task",
    cancelRun: "Cancel run",
    confirmCancel: "Cancel the running agent for {identifier}?",
    noAgents: "No agents are running.",
    noApprovals: "No approvals are waiting.",
    events: "Live events"
  }
};

export type TranslationDictionary = typeof en;

const sv: TranslationDictionary = {
  appName: "Symphony",
  common: {
    save: "Spara",
    saving: "Sparar…",
    cancel: "Avbryt",
    close: "Stäng",
    confirm: "Bekräfta",
    delete: "Ta bort",
    retry: "Försök igen",
    refresh: "Uppdatera",
    reload: "Läs in igen",
    copy: "Kopiera",
    copied: "Kopierat",
    search: "Sök",
    filter: "Filtrera",
    clearFilters: "Rensa filter",
    loading: "Laddar…",
    none: "Ingen",
    unknown: "Okänd",
    all: "Alla",
    yes: "Ja",
    no: "Nej",
    open: "Öppna",
    details: "Detaljer",
    back: "Tillbaka",
    next: "Nästa",
    create: "Skapa",
    edit: "Redigera",
    apply: "Tillämpa",
    discard: "Kasta",
    dismiss: "Stäng",
    showMore: "Visa mer",
    showLess: "Visa mindre",
    tokens: "tokens",
    justNow: "nyss",
    secondsAgo: "för {count} s sedan",
    minutesAgo: "för {count} min sedan",
    hoursAgo: "för {count} h sedan",
    daysAgo: "för {count} d sedan",
    durationSeconds: "{count} s",
    durationMinutes: "{minutes} min {seconds} s",
    durationHours: "{hours} h {minutes} min"
  },
  status: {
    backlog: "Backlogg",
    ready: "Redo",
    queued: "I kö",
    running: "Körs",
    blocked: "Blockerad",
    human_review: "Mänsklig granskning",
    completed: "Klar",
    failed: "Misslyckad",
    cancelled: "Avbruten"
  },
  phase: {
    preparing_workspace: "Förbereder arbetsyta",
    building_prompt: "Bygger prompt",
    launching_agent_process: "Startar agent",
    initializing_session: "Initierar session",
    streaming_turn: "Strömmar tur",
    finishing: "Avslutar",
    succeeded: "Lyckades",
    failed: "Misslyckades",
    timed_out: "Tidsgräns nådd",
    stalled: "Fastnat",
    canceled_by_reconciliation: "Avbruten"
  },
  provider: {
    codex: "OpenAI Codex",
    claude_code: "Claude Code",
    opencode: "OpenCode",
    simulated: "Simulerad agent"
  },
  authState: {
    authenticated: "Autentiserad",
    unauthenticated: "Inte inloggad",
    unknown: "Ej verifierad",
    not_required: "Krävs inte"
  },
  approvalDecision: {
    accept: "Godkänn en gång",
    accept_for_session: "Godkänn för sessionen",
    decline: "Neka",
    cancel: "Avbryt tur"
  },
  approvalStatus: {
    pending: "Väntar",
    approved: "Godkänd",
    denied: "Nekad",
    cancelled: "Avbruten"
  },
  transitionRejection: {
    same_status: "Uppgiften ligger redan i den här kolumnen.",
    use_dispatch: "Uppgifter börjar köras endast via start. Använd Starta i stället.",
    active_session: "En agentsession är aktiv. Avbryt den innan du ändrar status.",
    not_allowed: "Körtidens tillståndsmaskin tillåter inte flytt från {from} till {to}."
  },
  connection: {
    connecting: "Ansluter till körtiden…",
    connected: "Körtiden är ansluten",
    reconnecting: "Återansluter…",
    disconnected: "Körtiden är frånkopplad",
    unauthorized: "Autentisering krävs",
    paused: "Utskick pausat",
    live: "Live",
    polling: "Avsöker",
    stale: "Data kan vara inaktuell"
  },
  errors: {
    generic: "Något gick fel.",
    network: "Det gick inte att nå körtiden.",
    unauthorized: "Körtiden avvisade åtkomsttoken.",
    notFound: "Objektet finns inte längre.",
    conflict: "Begäran står i konflikt med nuvarande tillstånd.",
    validation: "Begäran avvisades: {message}"
  },
  mobile: {
    title: "Symphony Remote",
    connection: "Anslutning",
    hostUrl: "Körtidens URL",
    accessToken: "Åtkomsttoken",
    connect: "Anslut",
    disconnect: "Glöm anslutning",
    tokenStoredNotice: "Token sparas bara för den här sessionen om du inte väljer att komma ihåg den.",
    rememberToken: "Kom ihåg på den här enheten",
    activeAgents: "Aktiva agenter",
    pendingApprovals: "Väntande godkännanden",
    recentTasks: "Senaste uppgifter",
    newTask: "Ny uppgift",
    project: "Projekt",
    taskTitle: "Titel",
    taskDescription: "Beskrivning",
    dispatchNow: "Starta direkt",
    createTask: "Skapa uppgift",
    cancelRun: "Avbryt körning",
    confirmCancel: "Avbryta den körande agenten för {identifier}?",
    noAgents: "Inga agenter körs.",
    noApprovals: "Inga godkännanden väntar.",
    events: "Livehändelser"
  }
};

export const translations: Record<Locale, TranslationDictionary> = { en, sv };

export function getTranslations(locale: Locale): TranslationDictionary {
  return translations[locale] ?? translations.en;
}

export function isLocale(value: unknown): value is Locale {
  return value === "en" || value === "sv";
}
