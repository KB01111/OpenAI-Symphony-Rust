export type Locale = "en" | "sv";

export interface TranslationDictionary {
  appName: string;
  tagline: string;
  nav: {
    dashboard: string;
    projects: string;
    taskBoard: string;
    agentControl: string;
    executionView: string;
    prReview: string;
    workflowEditor: string;
    settings: string;
  };
  dashboard: {
    title: string;
    subtitle: string;
    activeAgents: string;
    runningTasks: string;
    queuedTasks: string;
    blockedTasks: string;
    completedTasks: string;
    failedTasks: string;
    tokenUsage: string;
    estimatedCost: string;
    cpuUsage: string;
    memoryUsage: string;
    powerState: string;
    recentPullRequests: string;
    liveEventFeed: string;
    refreshNow: string;
    pauseScheduler: string;
    resumeScheduler: string;
    noRecentEvents: string;
  };
  status: {
    backlog: string;
    ready: string;
    queued: string;
    running: string;
    blocked: string;
    human_review: string;
    completed: string;
    failed: string;
    cancelled: string;
  };
  tasks: {
    newTask: string;
    title: string;
    description: string;
    priority: string;
    provider: string;
    project: string;
    dispatchImmediately: string;
    createTask: string;
    cancelTask: string;
    retryTask: string;
    startTask: string;
    inspectExecution: string;
  };
  agents: {
    controlCenter: string;
    activeSessions: string;
    schedulerPausedNotice: string;
    providerCapabilities: string;
    openWorkspace: string;
    openDetails: string;
    cancelExecution: string;
    noActiveAgents: string;
    conciseMode: string;
    detailedLogsMode: string;
  };
  prReview: {
    title: string;
    mergeReadiness: string;
    approveMerge: string;
    humanApprovalRequired: string;
    ciStatus: string;
    securityWarnings: string;
    agentFeedbackEvaluation: string;
    changedFiles: string;
    diffPreview: string;
  };
  settings: {
    title: string;
    theme: string;
    language: string;
    closeBehavior: string;
    minimizeToTray: string;
    exitApplication: string;
    stopOrchestrator: string;
    acConcurrency: string;
    batteryConcurrency: string;
    remoteApi: string;
    exportDiagnostics: string;
    saveSettings: string;
    onboardingTitle: string;
  };
}

export const translations: Record<Locale, TranslationDictionary> = {
  en: {
    appName: "Symphony Desktop",
    tagline: "Native AI Coding Orchestration Platform",
    nav: {
      dashboard: "Dashboard",
      projects: "Projects",
      taskBoard: "Task Board",
      agentControl: "Agent Control Center",
      executionView: "Agent Execution",
      prReview: "Pull Request Review",
      workflowEditor: "WORKFLOW.md Studio",
      settings: "Settings"
    },
    dashboard: {
      title: "Orchestration Command Center",
      subtitle: "Real-time multi-agent dispatch, Git worktree isolation, and hardware-aware scheduling",
      activeAgents: "Active Agents",
      runningTasks: "Running Tasks",
      queuedTasks: "Queued Tasks",
      blockedTasks: "Blocked Tasks",
      completedTasks: "Completed Tasks",
      failedTasks: "Failed Tasks",
      tokenUsage: "Total Tokens",
      estimatedCost: "Estimated Cost",
      cpuUsage: "Host CPU",
      memoryUsage: "Host RAM",
      powerState: "Power Envelope",
      recentPullRequests: "Recent Pull Requests",
      liveEventFeed: "Real-Time Agent Telemetry",
      refreshNow: "Poll & Reconcile",
      pauseScheduler: "Pause Dispatching",
      resumeScheduler: "Resume Dispatching",
      noRecentEvents: "No agent events recorded yet. Create or dispatch a task to begin."
    },
    status: {
      backlog: "Backlog",
      ready: "Ready",
      queued: "Queued",
      running: "Running",
      blocked: "Blocked",
      human_review: "Human Review",
      completed: "Completed",
      failed: "Failed",
      cancelled: "Cancelled"
    },
    tasks: {
      newTask: "New Development Task",
      title: "Task Title",
      description: "Specification & Acceptance Criteria",
      priority: "Priority (1=Urgent, 4=Low)",
      provider: "Coding Agent Provider",
      project: "Target Project",
      dispatchImmediately: "Queue and dispatch immediately in isolated Git worktree",
      createTask: "Create Task",
      cancelTask: "Cancel Execution",
      retryTask: "Retry Task",
      startTask: "Dispatch Agent",
      inspectExecution: "Open Execution View"
    },
    agents: {
      controlCenter: "Agent Control Center",
      activeSessions: "Active Coding Agent Sessions",
      schedulerPausedNotice: "Scheduler dispatching is paused. Running sessions continue until completion or cancellation.",
      providerCapabilities: "Registered Provider Runtimes",
      openWorkspace: "Open Worktree",
      openDetails: "Inspect Session",
      cancelExecution: "Abort Session",
      noActiveAgents: "No agents are currently executing.",
      conciseMode: "Concise Progress",
      detailedLogsMode: "Detailed Protocol Logs"
    },
    prReview: {
      title: "Pull Request & Merge Readiness Review",
      mergeReadiness: "Evidence-Backed Merge Readiness (1-5)",
      approveMerge: "Approve & Complete Task",
      humanApprovalRequired: "Explicit human approval is enforced by default before completing or merging.",
      ciStatus: "CI & Validation Checks",
      securityWarnings: "Security & Policy Scan",
      agentFeedbackEvaluation: "Review Comment Verification",
      changedFiles: "Changed Files",
      diffPreview: "Unified Diff"
    },
    settings: {
      title: "Platform & Runtime Configuration",
      theme: "Appearance Theme",
      language: "Interface Language",
      closeBehavior: "Window Close Action",
      minimizeToTray: "Minimize to System Tray (Keep Orchestrator Running)",
      exitApplication: "Exit Application Completely",
      stopOrchestrator: "Stop Background Orchestrator",
      acConcurrency: "Max Concurrent Agents (AC Power)",
      batteryConcurrency: "Max Concurrent Agents (Battery)",
      remoteApi: "Authenticated Remote & iOS Companion API",
      exportDiagnostics: "Export Redacted Diagnostics",
      saveSettings: "Save Configuration",
      onboardingTitle: "First-Run Setup Wizard"
    }
  },
  sv: {
    appName: "Symphony Desktop",
    tagline: "Plattform för autonom AI-kodorkestrering",
    nav: {
      dashboard: "Översikt",
      projects: "Projekt",
      taskBoard: "Uppgiftstavla",
      agentControl: "Agentkontroll",
      executionView: "Agentkörning",
      prReview: "PR-granskning",
      workflowEditor: "WORKFLOW.md Studio",
      settings: "Inställningar"
    },
    dashboard: {
      title: "Orkestreringsöversikt",
      subtitle: "Realtidsstyrning av AI-agenter, isolerade Git-worktrees och hårdvarumedveten schemaläggning",
      activeAgents: "Aktiva agenter",
      runningTasks: "Pågående uppgifter",
      queuedTasks: "Köade uppgifter",
      blockedTasks: "Blockerade uppgifter",
      completedTasks: "Slutförda uppgifter",
      failedTasks: "Misslyckade uppgifter",
      tokenUsage: "Totala tokens",
      estimatedCost: "Uppskattad kostnad",
      cpuUsage: "Processorkraft",
      memoryUsage: "Minnesanvändning",
      powerState: "Strömläge",
      recentPullRequests: "Senaste Pull Requests",
      liveEventFeed: "Händelseström i realtid",
      refreshNow: "Synkronisera nu",
      pauseScheduler: "Pausa schemaläggare",
      resumeScheduler: "Återuppta schemaläggare",
      noRecentEvents: "Inga agenthändelser ännu. Skapa eller starta en uppgift för att börja."
    },
    status: {
      backlog: "Backlog",
      ready: "Redo",
      queued: "I kö",
      running: "Körs",
      blocked: "Blockerad",
      human_review: "Mänsklig granskning",
      completed: "Slutförd",
      failed: "Misslyckad",
      cancelled: "Avbruten"
    },
    tasks: {
      newTask: "Ny utvecklingsuppgift",
      title: "Titel på uppgift",
      description: "Specifikation och acceptanskriterier",
      priority: "Prioritet (1=Brådskande, 4=Låg)",
      provider: "AI-agentleverantör",
      project: "Målprojekt",
      dispatchImmediately: "Köa och starta omedelbart i isolerad Git-worktree",
      createTask: "Skapa uppgift",
      cancelTask: "Avbryt körning",
      retryTask: "Försök igen",
      startTask: "Starta agent",
      inspectExecution: "Öppna körningsvy"
    },
    agents: {
      controlCenter: "Agentkontrollcenter",
      activeSessions: "Aktiva kodagentsessioner",
      schedulerPausedNotice: "Schemaläggaren är pausad. Pågående sessioner fortsätter tills de är klara.",
      providerCapabilities: "Registrerade agentleverantörer",
      openWorkspace: "Öppna arbetsyta",
      openDetails: "Inspektera session",
      cancelExecution: "Avbryt session",
      noActiveAgents: "Inga agenter körs just nu.",
      conciseMode: "Koncis översikt",
      detailedLogsMode: "Detaljerade protokolloggar"
    },
    prReview: {
      title: "Granskning av Pull Request & Merge-beredskap",
      mergeReadiness: "Evidensbaserad Merge-beredskap (1-5)",
      approveMerge: "Godkänn & slutför uppgift",
      humanApprovalRequired: "Explicit mänskligt godkännande krävs alltid som standard före sammanslagning.",
      ciStatus: "CI- och valideringskontroller",
      securityWarnings: "Säkerhets- och policykontroll",
      agentFeedbackEvaluation: "Verifiering av granskningskommentarer",
      changedFiles: "Ändrade filer",
      diffPreview: "Diff-förhandsgranskning"
    },
    settings: {
      title: "Plattforms- och körningsinställningar",
      theme: "Tema",
      language: "Gränssnittsspråk",
      closeBehavior: "Åtgärd vid stängning av fönster",
      minimizeToTray: "Minimera till systemfältet (håll orkestreraren igång)",
      exitApplication: "Avsluta programmet helt",
      stopOrchestrator: "Stoppa bakgrundsorkestrerare",
      acConcurrency: "Max samtidiga agenter (Nätström)",
      batteryConcurrency: "Max samtidiga agenter (Batteri)",
      remoteApi: "Autentiserat fjärr-API & iOS-app",
      exportDiagnostics: "Exportera maskerad diagnostik",
      saveSettings: "Spara inställningar",
      onboardingTitle: "Startguide för Symphony Desktop"
    }
  }
};

export function getTranslations(locale: Locale = "en"): TranslationDictionary {
  return translations[locale] ?? translations.en;
}
