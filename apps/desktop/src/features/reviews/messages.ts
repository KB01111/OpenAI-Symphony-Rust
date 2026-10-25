import { defineMessages } from "../../lib/i18n.js";

/** Strings for the Pull Request Review Center. */
export const reviewMessages = defineMessages({
  en: {
    title: "Pull request reviews",
    description: "Evidence-backed merge readiness for agent-produced changes. Approval is always an explicit human decision.",
    listLabel: "Pull request reviews",
    count: "{count, plural, one {# review} other {# reviews}}",
    search: "Search reviews",
    searchPlaceholder: "Title, branch, #number, project…",
    filters: {
      approval: "Approval",
      all: "All",
      awaiting: "Awaiting approval",
      approved: "Approved",
      minScore: "Readiness score",
      anyScore: "Any score",
      scoreAtLeast: "Score ≥ {score}",
      hasBlockers: "Has blockers",
      clear: "Clear filters"
    },
    list: {
      score: "Readiness {score}/5",
      ci: "CI: {status}",
      ciUnknown: "CI: pending",
      ciNone: "No remote CI",
      localReview: "Local review",
      blockers: "{count, plural, one {# blocker} other {# blockers}}",
      unknownProject: "Unknown project",
      updated: "Updated"
    },
    empty: {
      noReviewsTitle: "No pull request reviews yet",
      noReviewsBody:
        "A review is produced automatically when an agent run completes: Symphony collects the branch diff, validation results, CI status and AI review comments, then scores merge readiness.",
      openRuns: "View agent runs",
      noMatchesTitle: "No reviews match these filters",
      noMatchesBody: "Adjust the search or clear the filters to see all reviews.",
      selectTitle: "Select a review",
      selectBody: "Choose a pull request on the left to inspect its evidence and record approval.",
      notFoundTitle: "Pull request review not found",
      notFoundBody: "It may belong to another project or have been removed from the runtime.",
      backToList: "Back to all reviews"
    },
    errors: {
      list: "Could not load pull request reviews",
      detail: "Could not load this pull request review"
    },
    detail: {
      repository: "Repository",
      pullRequest: "Pull request",
      url: "URL",
      branches: "Branches",
      task: "Task",
      openTask: "Open agent run",
      state: "State",
      mergeable: "Mergeable",
      conflicts: "Merge conflicts detected",
      securityTitle: "Security warnings",
      blockersTitle: "Blockers",
      descriptionTitle: "Description",
      localUrlHint: "Local worktree reference (not a web link)"
    },
    tabs: {
      label: "Review sections",
      files: "Files changed",
      diff: "Diff",
      validation: "Validation",
      comments: "AI review comments",
      security: "Security warnings"
    },
    files: {
      empty: "No changed files were recorded for this pull request.",
      status: "Status",
      showDiff: "Show diff for {path}",
      noHunk: "No diff hunk in the preview"
    },
    diff: {
      source: "Diff source",
      snapshot: "Review snapshot",
      worktree: "Current worktree diff",
      snapshotHint: "The review snapshot is captured when the run completes and may be capped.",
      empty: "No diff available.",
      truncated: "The diff was truncated by the runtime. Open the worktree for the full change.",
      filteredTo: "Showing {path}",
      clearFile: "Show all files",
      worktreeMissing: "The task's worktree no longer exists, so no live diff is available.",
      worktreeError: "Could not load the current worktree diff",
      fileNotInDiff: "{path} has no hunk in this diff."
    },
    validation: {
      empty: "No validation commands ran for this pull request.",
      passed: "Passed",
      failed: "Failed",
      exitCode: "Exit code {code}",
      attempt: "Attempt {number}",
      stdout: "Standard output",
      stderr: "Standard error",
      noOutput: "No output"
    },
    comments: {
      empty: "No AI review comments were recorded.",
      agent: "Agent",
      human: "Human",
      verified: "Verified against diff",
      unverified: "Not verified against diff",
      evaluation: "Evaluation",
      general: "General comment",
      fileDiff: "Diff for {path}",
      fileNotInDiff: "This file has no hunk in the review diff.",
      moreLines: "{count, plural, one {# more line} other {# more lines}} — see the Diff tab"
    },
    security: {
      empty: "No security warnings were raised."
    },
    publish: {
      button: "Create pull request",
      disabledConflicts: "Resolve the merge conflicts before opening a pull request.",
      disabledWarnings: "Resolve the security warnings before publishing this branch.",
      title: "Create a GitHub pull request?",
      body: "This commits the worktree's changes to branch {branch}, pushes that branch to the repository's origin and opens a pull request into {base}.",
      noMerge: "Nothing is merged and the task is not completed. Review and merge the pull request in your Git host.",
      confirm: "Create pull request",
      success: "Pull request opened",
      successBody: "Pull request #{number} was opened. Nothing was merged.",
      refresh: "Refresh status",
      refreshing: "Refreshing…",
      refreshHint: "Reads CI, merge state and comments from GitHub."
    },
    approve: {
      title: "Record human approval?",
      body: "This records your human approval for “{title}” and marks the task as completed.",
      noMerge: "It does NOT merge, push or close anything. Merging remains a separate action in your Git host.",
      lowScore: "The readiness score is {score}/5 and there are open blockers. Review them before approving.",
      confirm: "Record approval",
      success: "Approval recorded",
      successBody: "The task was marked completed. Nothing was merged."
    }
  },
  sv: {
    title: "Granskning av pull requests",
    description: "Evidensbaserad sammanslagningsberedskap för agentproducerade ändringar. Godkännande är alltid ett uttryckligt mänskligt beslut.",
    listLabel: "Granskningar av pull requests",
    count: "{count, plural, one {# granskning} other {# granskningar}}",
    search: "Sök granskningar",
    searchPlaceholder: "Titel, gren, #nummer, projekt…",
    filters: {
      approval: "Godkännande",
      all: "Alla",
      awaiting: "Väntar på godkännande",
      approved: "Godkända",
      minScore: "Beredskapspoäng",
      anyScore: "Alla poäng",
      scoreAtLeast: "Poäng ≥ {score}",
      hasBlockers: "Har blockerare",
      clear: "Rensa filter"
    },
    list: {
      score: "Beredskap {score}/5",
      ci: "CI: {status}",
      ciUnknown: "CI: väntar",
      ciNone: "Ingen fjärr-CI",
      localReview: "Lokal granskning",
      blockers: "{count} blockerare",
      unknownProject: "Okänt projekt",
      updated: "Uppdaterad"
    },
    empty: {
      noReviewsTitle: "Inga granskningar av pull requests ännu",
      noReviewsBody:
        "En granskning skapas automatiskt när en agentkörning slutförs: Symphony samlar grenens diff, valideringsresultat, CI-status och AI-granskningskommentarer och poängsätter sedan sammanslagningsberedskapen.",
      openRuns: "Visa agentkörningar",
      noMatchesTitle: "Inga granskningar matchar filtren",
      noMatchesBody: "Ändra sökningen eller rensa filtren för att se alla granskningar.",
      selectTitle: "Välj en granskning",
      selectBody: "Välj en pull request till vänster för att granska evidensen och registrera godkännande.",
      notFoundTitle: "Granskningen hittades inte",
      notFoundBody: "Den kan tillhöra ett annat projekt eller ha tagits bort från körmiljön.",
      backToList: "Tillbaka till alla granskningar"
    },
    errors: {
      list: "Det gick inte att läsa in granskningarna",
      detail: "Det gick inte att läsa in granskningen"
    },
    detail: {
      repository: "Kodförråd",
      pullRequest: "Pull request",
      url: "URL",
      branches: "Grenar",
      task: "Uppgift",
      openTask: "Öppna agentkörning",
      state: "Tillstånd",
      mergeable: "Sammanslagningsbar",
      conflicts: "Sammanslagningskonflikter upptäcktes",
      securityTitle: "Säkerhetsvarningar",
      blockersTitle: "Blockerare",
      descriptionTitle: "Beskrivning",
      localUrlHint: "Lokal referens till arbetsträd (ingen webblänk)"
    },
    tabs: {
      label: "Granskningsavsnitt",
      files: "Ändrade filer",
      diff: "Diff",
      validation: "Validering",
      comments: "AI-granskningskommentarer",
      security: "Säkerhetsvarningar"
    },
    files: {
      empty: "Inga ändrade filer registrerades för denna pull request.",
      status: "Status",
      showDiff: "Visa diff för {path}",
      noHunk: "Ingen diff-del i förhandsvisningen"
    },
    diff: {
      source: "Diff-källa",
      snapshot: "Granskningsögonblicksbild",
      worktree: "Aktuell diff i arbetsträdet",
      snapshotHint: "Ögonblicksbilden tas när körningen slutförs och kan vara begränsad i storlek.",
      empty: "Ingen diff tillgänglig.",
      truncated: "Diffen kortades av körmiljön. Öppna arbetsträdet för hela ändringen.",
      filteredTo: "Visar {path}",
      clearFile: "Visa alla filer",
      worktreeMissing: "Uppgiftens arbetsträd finns inte längre, så ingen aktuell diff är tillgänglig.",
      worktreeError: "Det gick inte att läsa in arbetsträdets aktuella diff",
      fileNotInDiff: "{path} har ingen del i denna diff."
    },
    validation: {
      empty: "Inga valideringskommandon kördes för denna pull request.",
      passed: "Godkänd",
      failed: "Misslyckad",
      exitCode: "Slutkod {code}",
      attempt: "Försök {number}",
      stdout: "Standardutdata",
      stderr: "Standardfel",
      noOutput: "Ingen utdata"
    },
    comments: {
      empty: "Inga AI-granskningskommentarer registrerades.",
      agent: "Agent",
      human: "Människa",
      verified: "Verifierad mot diff",
      unverified: "Inte verifierad mot diff",
      evaluation: "Bedömning",
      general: "Allmän kommentar",
      fileDiff: "Diff för {path}",
      fileNotInDiff: "Filen har ingen del i granskningens diff.",
      moreLines: "{count, plural, one {# rad} other {# rader}} till — se fliken Diff"
    },
    security: {
      empty: "Inga säkerhetsvarningar rapporterades."
    },
    publish: {
      button: "Skapa pull request",
      disabledConflicts: "Lös sammanslagningskonflikterna innan du öppnar en pull request.",
      disabledWarnings: "Åtgärda säkerhetsvarningarna innan grenen publiceras.",
      title: "Skapa en GitHub-pull request?",
      body: "Detta committar arbetsträdets ändringar till grenen {branch}, pushar grenen till repots origin och öppnar en pull request mot {base}.",
      noMerge: "Inget slås samman och uppgiften slutförs inte. Granska och slå samman pull requesten i din Git-tjänst.",
      confirm: "Skapa pull request",
      success: "Pull request öppnad",
      successBody: "Pull request #{number} öppnades. Inget slogs samman.",
      refresh: "Uppdatera status",
      refreshing: "Uppdaterar…",
      refreshHint: "Läser CI, sammanslagningsstatus och kommentarer från GitHub."
    },
    approve: {
      title: "Registrera mänskligt godkännande?",
      body: "Detta registrerar ditt mänskliga godkännande av ”{title}” och markerar uppgiften som slutförd.",
      noMerge: "Inget slås samman, pushas eller stängs. Sammanslagning är fortfarande en separat åtgärd i din Git-tjänst.",
      lowScore: "Beredskapspoängen är {score}/5 och det finns öppna blockerare. Granska dem innan du godkänner.",
      confirm: "Registrera godkännande",
      success: "Godkännande registrerat",
      successBody: "Uppgiften markerades som slutförd. Inget slogs samman."
    }
  }
});

export type ReviewMessages = (typeof reviewMessages)["en"];
