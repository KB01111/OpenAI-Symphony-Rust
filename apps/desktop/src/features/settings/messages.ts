import { defineMessages } from "../../lib/i18n.js";

export const settingsMessages = defineMessages({
  en: {
    title: "Settings",
    description: "Runtime and desktop preferences. Saved settings are stored by the runtime.",
    sectionsNav: "Settings sections",
    loadError: "Could not load settings",
    unsaved: "Unsaved changes",
    invalidCount: "{count} field(s) need attention",
    save: "Save changes",
    reset: "Reset",
    saved: "Settings saved",
    saveFailed: "The runtime rejected the settings",
    sections: {
      appearance: "Appearance",
      desktop: "Desktop behaviour",
      scheduling: "Scheduling & resources",
      defaults: "Default provider",
      remote: "Remote access",
      integrations: "Integrations",
      diagnostics: "Diagnostics",
      application: "Application"
    },
    appearance: {
      intro: "Theme and language apply immediately and are saved to the runtime. Density is stored on this device only.",
      theme: "Theme",
      themeSystem: "Follow Windows",
      themeDark: "Dark",
      themeLight: "Light",
      language: "Language",
      english: "English",
      swedish: "Svenska",
      density: "Density",
      compact: "Compact",
      comfortable: "Comfortable",
      densityHint: "Local to this device."
    },
    desktop: {
      intro: "How the Symphony window and background runtime behave on Windows.",
      tray: "Minimize to the notification area when the window is closed",
      trayHint: "Agents keep running in the background. Use Exit Symphony to stop everything.",
      autostart: "Start the background runtime when I sign in to Windows",
      autostartHint: "Applied through the desktop host when you save.",
      notDesktop:
        "You are not running inside the Symphony desktop host. These preferences are saved to the runtime but only take effect in Symphony Desktop."
    },
    scheduling: {
      intro: "How often trackers are polled and how many agents may run at once. The runtime lowers concurrency further under load.",
      polling: "Polling interval (ms)",
      pollingHint: "At least 2000 ms.",
      maxAc: "Max concurrent agents on AC power",
      maxAcHint: "1–32.",
      maxBattery: "Max concurrent agents on battery",
      maxBatteryHint: "Cannot exceed the AC limit. 0 pauses dispatching on battery.",
      memory: "High memory threshold (%)",
      cpu: "High CPU threshold (%)",
      thresholdHint: "10–100. Dispatching is delayed above this load.",
      thermal: "Reduce concurrency while the CPU is thermally throttled",
      live: "Current host",
      liveUnavailable: "Live hardware data is unavailable.",
      power: "Power",
      ac: "AC power",
      battery: "Battery {percent}%",
      batteryUnknown: "Battery",
      powerMode: "Power mode",
      cpuLoad: "CPU load",
      memoryLoad: "Memory load",
      recommended: "Recommended concurrency",
      effective: "Effective concurrency",
      throttled: "Thermal throttling",
      delayed: "Dispatch delayed: {reason}",
      notDelayed: "Dispatching is not delayed."
    },
    defaults: {
      intro: "Used for new tasks when neither the task nor its project selects a provider or model.",
      provider: "Default provider",
      model: "Default model",
      modelHint: "Leave empty to use the provider's own default. Any model id is accepted.",
      notInstalled: "not installed",
      manage: "Manage providers"
    },
    remote: {
      intro: "Optional authenticated listener for the Symphony Remote companion.",
      enabled: "Enable remote API",
      bind: "Bind address",
      bindHint: "127.0.0.1 keeps the listener local. Any other address exposes it on your network.",
      port: "Port",
      warningTitle: "Remote access requirements",
      warning:
        "The remote listener only starts when a SYMPHONY_REMOTE_TOKEN of at least 16 characters is stored in the runtime credential vault. It serves plain HTTP: put a TLS-terminating reverse proxy in front of it before exposing it beyond this machine. Changes apply the next time the runtime starts.",
      exposed: "This address is reachable from other devices on your network."
    },
    integrations: {
      intro: "Tracker tokens used by host-side tools (GitHub Issues, Linear). Agents never receive these values.",
      providerKeys: "Provider API keys (OpenAI, Anthropic) are managed on the Providers page.",
      openProviders: "Open Providers",
      loadError: "Could not load credentials",
      descriptions: {
        GITHUB_TOKEN: "GitHub Issues tracker and pull request tools.",
        GH_TOKEN: "Alternative GitHub token name used by the gh CLI.",
        GITHUB_ENTERPRISE_TOKEN: "GitHub Enterprise Server.",
        LINEAR_API_KEY: "Linear tracker.",
        OPENAI_API_KEY: "OpenAI Codex.",
        ANTHROPIC_API_KEY: "Claude Code."
      } as Record<string, string>
    },
    diagnostics: {
      intro: "Export a redacted JSON bundle (settings, runtime state, recent errors) to attach to a bug report. Secrets are masked by the runtime before export.",
      export: "Export diagnostics",
      exporting: "Exporting…",
      ready: "Diagnostics ready ({size}).",
      download: "Download symphony-diagnostics.json",
      failed: "Export failed"
    },
    application: {
      intro: "Runtime this window is connected to.",
      service: "Service",
      version: "Version",
      apiVersion: "API version",
      mode: "Connection mode",
      baseUrl: "Endpoint",
      startedAt: "Runtime started",
      auth: "Authentication",
      authRequired: "Bearer token required",
      authNone: "Not required",
      loadError: "Could not load runtime information",
      modes: {
        embedded: "Embedded in the desktop app",
        attached: "Attached to an external runtime",
        dev: "Development server",
        remote: "Remote runtime"
      } as Record<string, string>,
      exit: "Exit Symphony completely",
      exitTitle: "Exit Symphony?",
      exitDescription:
        "Closes the window and stops the background runtime. Running agents are interrupted and resume only when Symphony starts again.",
      exitConfirm: "Exit"
    },
    validation: {
      number: "Enter a number.",
      integer: "Enter a whole number.",
      polling: "Must be at least 2000 ms.",
      maxAc: "Must be between 1 and 32.",
      maxBattery: "Must be between 0 and the AC limit ({max}).",
      threshold: "Must be between 10 and 100.",
      port: "Must be a TCP port between 1 and 65535.",
      bind: "Enter a bind address.",
      portInUse: "This is the local API's port ({port}); choose a different remote port."
    }
  },
  sv: {
    title: "Inställningar",
    description: "Inställningar för körtid och skrivbord. Sparade inställningar lagras av körtiden.",
    sectionsNav: "Inställningsavsnitt",
    loadError: "Det gick inte att läsa in inställningarna",
    unsaved: "Osparade ändringar",
    invalidCount: "{count} fält behöver åtgärdas",
    save: "Spara ändringar",
    reset: "Återställ",
    saved: "Inställningarna sparades",
    saveFailed: "Körtiden avvisade inställningarna",
    sections: {
      appearance: "Utseende",
      desktop: "Skrivbordsbeteende",
      scheduling: "Schemaläggning och resurser",
      defaults: "Standardleverantör",
      remote: "Fjärråtkomst",
      integrations: "Integrationer",
      diagnostics: "Diagnostik",
      application: "Program"
    },
    appearance: {
      intro: "Tema och språk tillämpas direkt och sparas i körtiden. Täthet sparas bara på den här enheten.",
      theme: "Tema",
      themeSystem: "Följ Windows",
      themeDark: "Mörkt",
      themeLight: "Ljust",
      language: "Språk",
      english: "English",
      swedish: "Svenska",
      density: "Täthet",
      compact: "Kompakt",
      comfortable: "Bekväm",
      densityHint: "Gäller bara den här enheten."
    },
    desktop: {
      intro: "Hur Symphony-fönstret och bakgrundskörtiden beter sig i Windows.",
      tray: "Minimera till meddelandefältet när fönstret stängs",
      trayHint: "Agenter fortsätter köras i bakgrunden. Använd Avsluta Symphony för att stoppa allt.",
      autostart: "Starta bakgrundskörtiden när jag loggar in i Windows",
      autostartHint: "Tillämpas via skrivbordsvärden när du sparar.",
      notDesktop:
        "Du kör inte i Symphonys skrivbordsvärd. Inställningarna sparas i körtiden men gäller bara i Symphony Desktop."
    },
    scheduling: {
      intro: "Hur ofta spårare avsöks och hur många agenter som får köras samtidigt. Körtiden sänker samtidigheten ytterligare vid hög belastning.",
      polling: "Avsökningsintervall (ms)",
      pollingHint: "Minst 2000 ms.",
      maxAc: "Max samtidiga agenter på nätström",
      maxAcHint: "1–32.",
      maxBattery: "Max samtidiga agenter på batteri",
      maxBatteryHint: "Får inte överstiga gränsen för nätström. 0 pausar utskick på batteri.",
      memory: "Tröskel för hög minnesanvändning (%)",
      cpu: "Tröskel för hög CPU-last (%)",
      thresholdHint: "10–100. Utskick fördröjs över denna belastning.",
      thermal: "Minska samtidigheten när processorn stryps termiskt",
      live: "Aktuell värd",
      liveUnavailable: "Live-data om hårdvaran saknas.",
      power: "Ström",
      ac: "Nätström",
      battery: "Batteri {percent} %",
      batteryUnknown: "Batteri",
      powerMode: "Energiläge",
      cpuLoad: "CPU-last",
      memoryLoad: "Minnesanvändning",
      recommended: "Rekommenderad samtidighet",
      effective: "Faktisk samtidighet",
      throttled: "Termisk strypning",
      delayed: "Utskick fördröjt: {reason}",
      notDelayed: "Utskick är inte fördröjt."
    },
    defaults: {
      intro: "Används för nya uppgifter när varken uppgiften eller projektet väljer leverantör eller modell.",
      provider: "Standardleverantör",
      model: "Standardmodell",
      modelHint: "Lämna tomt för att använda leverantörens egen standard. Alla modell-id:n accepteras.",
      notInstalled: "inte installerad",
      manage: "Hantera leverantörer"
    },
    remote: {
      intro: "Valfri autentiserad lyssnare för följeappen Symphony Remote.",
      enabled: "Aktivera fjärr-API",
      bind: "Bindningsadress",
      bindHint: "127.0.0.1 håller lyssnaren lokal. Alla andra adresser exponerar den i ditt nätverk.",
      port: "Port",
      warningTitle: "Krav för fjärråtkomst",
      warning:
        "Fjärrlyssnaren startar bara när en SYMPHONY_REMOTE_TOKEN på minst 16 tecken finns i körtidens valv. Den använder okrypterad HTTP: placera en omvänd proxy med TLS framför den innan den exponeras utanför datorn. Ändringar gäller nästa gång körtiden startar.",
      exposed: "Adressen kan nås från andra enheter i ditt nätverk."
    },
    integrations: {
      intro: "Spårartoken som används av verktyg på värden (GitHub Issues, Linear). Agenter får aldrig dessa värden.",
      providerKeys: "API-nycklar för leverantörer (OpenAI, Anthropic) hanteras på sidan Leverantörer.",
      openProviders: "Öppna Leverantörer",
      loadError: "Det gick inte att läsa in autentiseringsuppgifter",
      descriptions: {
        GITHUB_TOKEN: "GitHub Issues-spårare och verktyg för pull requests.",
        GH_TOKEN: "Alternativt GitHub-tokennamn som används av gh CLI.",
        GITHUB_ENTERPRISE_TOKEN: "GitHub Enterprise Server.",
        LINEAR_API_KEY: "Linear-spårare.",
        OPENAI_API_KEY: "OpenAI Codex.",
        ANTHROPIC_API_KEY: "Claude Code."
      } as Record<string, string>
    },
    diagnostics: {
      intro: "Exportera ett maskerat JSON-paket (inställningar, körtidstillstånd, senaste fel) att bifoga en felrapport. Hemligheter maskeras av körtiden före export.",
      export: "Exportera diagnostik",
      exporting: "Exporterar…",
      ready: "Diagnostiken är klar ({size}).",
      download: "Ladda ned symphony-diagnostics.json",
      failed: "Exporten misslyckades"
    },
    application: {
      intro: "Körtiden som fönstret är anslutet till.",
      service: "Tjänst",
      version: "Version",
      apiVersion: "API-version",
      mode: "Anslutningsläge",
      baseUrl: "Slutpunkt",
      startedAt: "Körtiden startades",
      auth: "Autentisering",
      authRequired: "Bearer-token krävs",
      authNone: "Krävs inte",
      loadError: "Det gick inte att läsa in information om körtiden",
      modes: {
        embedded: "Inbäddad i skrivbordsappen",
        attached: "Ansluten till extern körtid",
        dev: "Utvecklingsserver",
        remote: "Fjärrkörtid"
      } as Record<string, string>,
      exit: "Avsluta Symphony helt",
      exitTitle: "Avsluta Symphony?",
      exitDescription:
        "Stänger fönstret och stoppar bakgrundskörtiden. Körande agenter avbryts och återupptas först när Symphony startar igen.",
      exitConfirm: "Avsluta"
    },
    validation: {
      number: "Ange ett tal.",
      integer: "Ange ett heltal.",
      polling: "Måste vara minst 2000 ms.",
      maxAc: "Måste vara mellan 1 och 32.",
      maxBattery: "Måste vara mellan 0 och gränsen för nätström ({max}).",
      threshold: "Måste vara mellan 10 och 100.",
      port: "Måste vara en TCP-port mellan 1 och 65535.",
      bind: "Ange en bindningsadress.",
      portInUse: "Det här är det lokala API:ets port ({port}); välj en annan fjärrport."
    }
  }
});

export type SettingsMessages = (typeof settingsMessages)["en"];
