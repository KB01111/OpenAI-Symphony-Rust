import { defineMessages } from "../../lib/i18n.js";

export const overviewMessages = defineMessages({
  en: {
    title: "Overview",
    description: "Real-time orchestration across your repositories",
    metrics: {
      activeAgents: "Active agents",
      queued: "Queued tasks",
      approvals: "Pending approvals",
      failed: "Failed runs",
      completed: "Completed tasks",
      repositories: "Active repositories",
      tokens: "Token usage",
      cost: "Estimated cost",
      health: "Runtime health",
      healthy: "Healthy",
      degraded: "Degraded",
      paused: "Paused",
      retrying: "{count} retrying",
      reviewWaiting: "{count} awaiting review",
      ofRepositories: "of {count} registered",
      runningFor: "{duration} agent time",
      capacity: "capacity {count}"
    },
    live: {
      title: "Live agent activity",
      empty: "No agents are running",
      emptyDetail: "Dispatch a task to watch agents work here in real time.",
      newTask: "New task",
      task: "Task",
      project: "Project",
      provider: "Provider",
      phase: "Phase",
      operation: "Current operation",
      duration: "Duration",
      tokens: "Tokens",
      lastActivity: "Last activity",
      open: "Open execution for {identifier}"
    },
    approvals: {
      title: "Pending approvals",
      empty: "No approvals are waiting",
      openTask: "Open task"
    },
    timeline: {
      title: "Recent activity",
      empty: "No activity recorded yet",
      emptyDetail: "Runtime events from all repositories appear here."
    },
    onboarding: {
      title: "Register your first repository",
      detail: "Symphony runs coding agents in isolated Git worktrees of registered repositories.",
      action: "Add repository"
    }
  },
  sv: {
    title: "Översikt",
    description: "Orkestrering i realtid över dina repositories",
    metrics: {
      activeAgents: "Aktiva agenter",
      queued: "Uppgifter i kö",
      approvals: "Väntande godkännanden",
      failed: "Misslyckade körningar",
      completed: "Klara uppgifter",
      repositories: "Aktiva repositories",
      tokens: "Tokenanvändning",
      cost: "Uppskattad kostnad",
      health: "Körtidens hälsa",
      healthy: "Frisk",
      degraded: "Försämrad",
      paused: "Pausad",
      retrying: "{count} försöker igen",
      reviewWaiting: "{count} väntar på granskning",
      ofRepositories: "av {count} registrerade",
      runningFor: "{duration} agenttid",
      capacity: "kapacitet {count}"
    },
    live: {
      title: "Live agentaktivitet",
      empty: "Inga agenter körs",
      emptyDetail: "Starta en uppgift för att följa agenterna i realtid här.",
      newTask: "Ny uppgift",
      task: "Uppgift",
      project: "Projekt",
      provider: "Leverantör",
      phase: "Fas",
      operation: "Aktuell åtgärd",
      duration: "Varaktighet",
      tokens: "Tokens",
      lastActivity: "Senaste aktivitet",
      open: "Öppna körning för {identifier}"
    },
    approvals: {
      title: "Väntande godkännanden",
      empty: "Inga godkännanden väntar",
      openTask: "Öppna uppgift"
    },
    timeline: {
      title: "Senaste aktivitet",
      empty: "Ingen aktivitet registrerad ännu",
      emptyDetail: "Körtidshändelser från alla repositories visas här."
    },
    onboarding: {
      title: "Registrera ditt första repository",
      detail: "Symphony kör kodagenter i isolerade Git-arbetsträd för registrerade repositories.",
      action: "Lägg till repository"
    }
  }
});
