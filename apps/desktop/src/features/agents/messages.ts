import { defineMessages } from "../../lib/i18n.js";
import type { AgentGroupBy, AgentSegment } from "./grouping.js";

const enSegments: Record<AgentSegment, string> = {
  running: "Running",
  approval: "Waiting for approval",
  retrying: "Retrying",
  queued: "Queued",
  failed: "Failed",
  cancelled: "Cancelled",
  completed: "Completed"
};

const svSegments: Record<AgentSegment, string> = {
  running: "Körs",
  approval: "Väntar på godkännande",
  retrying: "Försöker igen",
  queued: "I kö",
  failed: "Misslyckade",
  cancelled: "Avbrutna",
  completed: "Slutförda"
};

const enGroupBy: Record<AgentGroupBy, string> = { status: "Status", provider: "Provider", project: "Project" };
const svGroupBy: Record<AgentGroupBy, string> = { status: "Status", provider: "Leverantör", project: "Projekt" };

/** Strings for the Agent Runs control center (`/runs`). */
export const agentsMessages = defineMessages({
  en: {
    title: "Agent runs",
    description: "{running} running · {approval} waiting for approval · {retrying} retrying · {queued} queued",
    segments: enSegments,
    groupBy: enGroupBy,
    groupByLabel: "Group by",
    all: "All",
    segmentTabs: "Filter agent runs by state",
    recentHint: "Finished runs from the last 24 hours.",
    list: "Agent runs",
    groupCount: "{count} run(s)",
    unknownProvider: "Unknown provider",
    row: {
      unknownTitle: "Task details unavailable",
      unknownProject: "Unknown project",
      operation: "Current operation",
      idle: "Waiting for the next event…",
      duration: "Duration",
      tokens: "{count} tokens",
      tokensLabel: "Tokens",
      cost: "Estimated cost",
      turns: "{count} turn(s)",
      turnsLabel: "Turns",
      attemptLabel: "Attempt",
      lastEvent: "Last activity",
      attempt: "Attempt {number}",
      retryDue: "Retry due",
      pendingApprovals: "{count} pending approval(s)",
      finished: "Finished",
      error: "Last error"
    },
    actions: {
      label: "Actions for {identifier}",
      openExecution: "Open live execution",
      cancel: "Cancel run",
      retry: "Retry",
      dispatch: "Start now"
    },
    cancelDialog: {
      title: "Cancel {identifier}?",
      description: "The agent session is stopped and the task moves to Cancelled. Changes already in the worktree are kept.",
      confirm: "Cancel run"
    },
    empty: {
      title: "No agent runs yet",
      description: "Start or queue a task and its agent session appears here in real time.",
      cta: "Go to Tasks",
      filtered: "Nothing in this group right now."
    },
    loadFailed: "Could not load agent runs"
  },
  sv: {
    title: "Agentkörningar",
    description: "{running} körs · {approval} väntar på godkännande · {retrying} försöker igen · {queued} i kö",
    segments: svSegments,
    groupBy: svGroupBy,
    groupByLabel: "Gruppera efter",
    all: "Alla",
    segmentTabs: "Filtrera agentkörningar efter tillstånd",
    recentHint: "Avslutade körningar från de senaste 24 timmarna.",
    list: "Agentkörningar",
    groupCount: "{count} körning(ar)",
    unknownProvider: "Okänd leverantör",
    row: {
      unknownTitle: "Uppgiftsinformation saknas",
      unknownProject: "Okänt projekt",
      operation: "Aktuell åtgärd",
      idle: "Väntar på nästa händelse…",
      duration: "Varaktighet",
      tokens: "{count} tokens",
      tokensLabel: "Tokens",
      cost: "Uppskattad kostnad",
      turns: "{count} tur(er)",
      turnsLabel: "Turer",
      attemptLabel: "Försök",
      lastEvent: "Senaste aktivitet",
      attempt: "Försök {number}",
      retryDue: "Nytt försök",
      pendingApprovals: "{count} väntande godkännande(n)",
      finished: "Avslutad",
      error: "Senaste fel"
    },
    actions: {
      label: "Åtgärder för {identifier}",
      openExecution: "Öppna livekörning",
      cancel: "Avbryt körning",
      retry: "Försök igen",
      dispatch: "Starta nu"
    },
    cancelDialog: {
      title: "Avbryta {identifier}?",
      description: "Agentsessionen stoppas och uppgiften flyttas till Avbruten. Ändringar som redan finns i arbetsträdet behålls.",
      confirm: "Avbryt körning"
    },
    empty: {
      title: "Inga agentkörningar ännu",
      description: "Starta eller köa en uppgift så visas dess agentsession här i realtid.",
      cta: "Gå till Uppgifter",
      filtered: "Inget i den här gruppen just nu."
    },
    loadFailed: "Det gick inte att läsa in agentkörningar"
  }
});
