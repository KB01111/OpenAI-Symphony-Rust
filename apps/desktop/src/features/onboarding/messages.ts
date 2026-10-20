import { defineMessages } from "../../lib/i18n.js";

/** Strings for the first-run onboarding dialog. */
export const onboardingMessages = defineMessages({
  en: {
    title: "Welcome to Symphony",
    stepOf: "Step {step} of {total}",
    steps: { welcome: "Welcome", providers: "Agents", repository: "Repository", finish: "Ready" },
    welcome: {
      heading: "Orchestrate coding agents on your own machine",
      body: "Symphony runs Codex, Claude Code and OpenCode in isolated Git worktrees, streams their work live, asks you before risky commands and scores pull requests for merge readiness. It never merges on its own.",
      points: [
        "Each task gets its own worktree; your checkout is never modified.",
        "Commands and file changes can require your approval.",
        "Everything runs locally; the API is bound to this machine and token-protected."
      ]
    },
    providers: {
      heading: "Detected coding agents",
      body: "Symphony looked for agent CLIs on this machine. You can install or sign in later from Providers.",
      installed: "Installed",
      missing: "Not installed",
      loading: "Detecting agents…",
      failed: "Agent detection failed",
      none: "No agent CLI was found. The built-in simulated agent still lets you try Symphony safely."
    },
    repository: {
      heading: "Register a repository",
      body: "Agents work in Git repositories you register. Symphony creates isolated worktrees inside its own workspace folder.",
      add: "Add repository…",
      registered: "Registered repositories",
      none: "No repository registered yet.",
      later: "You can also add repositories later from Projects."
    },
    finish: {
      heading: "You're ready",
      body: "Create a task with Ctrl+N, watch agents in Agent runs and review results in Pull requests. Press Ctrl+K at any time for the command palette."
    },
    actions: { back: "Back", next: "Next", skip: "Skip setup", finish: "Start using Symphony", finishing: "Saving…" },
    saveFailed: "Could not save onboarding progress"
  },
  sv: {
    title: "Välkommen till Symphony",
    stepOf: "Steg {step} av {total}",
    steps: { welcome: "Välkommen", providers: "Agenter", repository: "Repository", finish: "Klart" },
    welcome: {
      heading: "Orkestrera kodagenter på din egen dator",
      body: "Symphony kör Codex, Claude Code och OpenCode i isolerade Git-arbetsträd, strömmar deras arbete live, frågar dig före riskfyllda kommandon och bedömer pull requests för sammanslagning. Den slår aldrig ihop något på egen hand.",
      points: [
        "Varje uppgift får ett eget arbetsträd; din utcheckning ändras aldrig.",
        "Kommandon och filändringar kan kräva ditt godkännande.",
        "Allt körs lokalt; API:et är bundet till den här datorn och skyddat med token."
      ]
    },
    providers: {
      heading: "Hittade kodagenter",
      body: "Symphony letade efter agent-CLI:er på den här datorn. Du kan installera eller logga in senare under Leverantörer.",
      installed: "Installerad",
      missing: "Inte installerad",
      loading: "Letar efter agenter…",
      failed: "Agentidentifieringen misslyckades",
      none: "Ingen agent-CLI hittades. Den inbyggda simulerade agenten låter dig ändå prova Symphony säkert."
    },
    repository: {
      heading: "Registrera ett repository",
      body: "Agenter arbetar i Git-repositories som du registrerar. Symphony skapar isolerade arbetsträd i sin egen arbetsmapp.",
      add: "Lägg till repository…",
      registered: "Registrerade repositories",
      none: "Inget repository registrerat ännu.",
      later: "Du kan också lägga till repositories senare under Projekt."
    },
    finish: {
      heading: "Du är redo",
      body: "Skapa en uppgift med Ctrl+N, följ agenter under Agentkörningar och granska resultat under Pull requests. Tryck Ctrl+K när som helst för kommandopaletten."
    },
    actions: { back: "Tillbaka", next: "Nästa", skip: "Hoppa över", finish: "Börja använda Symphony", finishing: "Sparar…" },
    saveFailed: "Kunde inte spara introduktionen"
  }
});
