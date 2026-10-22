import { defineMessages } from "../../lib/i18n.js";

export const updateMessages = defineMessages({
  en: {
    bannerLabel: "Application update",
    available: "Version {version} is available (you are running {current}).",
    availablePre: "Pre-release {version} is available (you are running {current}).",
    install: "Update now",
    openReleases: "Open release page",
    openReleasesFailed: "Could not open the release page",
    manualOnly: "This release cannot be installed from inside the app. Download the installer from the release page.",
    later: "Not now",
    downloading: "Downloading update… {percent}%",
    downloadingUnknown: "Downloading update…",
    progressLabel: "Update download",
    installing: "Installing. Symphony closes and starts again when the update is done.",
    installFailed: "The update could not be installed",
    retry: "Try again",
    confirmTitle: "Install the update now?",
    confirmDescription:
      "{count} agent run(s) are in progress. Installing closes Symphony and interrupts them; they are queued again and resume when Symphony starts.",
    confirmInstall: "Interrupt and install"
  },
  sv: {
    bannerLabel: "Programuppdatering",
    available: "Version {version} finns tillgänglig (du kör {current}).",
    availablePre: "Förhandsversionen {version} finns tillgänglig (du kör {current}).",
    install: "Uppdatera nu",
    openReleases: "Öppna versionssidan",
    openReleasesFailed: "Det gick inte att öppna versionssidan",
    manualOnly: "Den här versionen kan inte installeras inifrån appen. Hämta installationsprogrammet från versionssidan.",
    later: "Inte nu",
    downloading: "Laddar ned uppdateringen… {percent} %",
    downloadingUnknown: "Laddar ned uppdateringen…",
    progressLabel: "Nedladdning av uppdatering",
    installing: "Installerar. Symphony stängs och startar igen när uppdateringen är klar.",
    installFailed: "Uppdateringen kunde inte installeras",
    retry: "Försök igen",
    confirmTitle: "Installera uppdateringen nu?",
    confirmDescription:
      "{count} agentkörning(ar) pågår. Installationen stänger Symphony och avbryter dem; de köas igen och fortsätter när Symphony startar.",
    confirmInstall: "Avbryt och installera"
  }
});
