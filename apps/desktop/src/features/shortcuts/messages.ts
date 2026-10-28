import { defineMessages } from "../../lib/i18n.js";

export const shortcutsMessages = defineMessages({
  en: {
    title: "Keyboard shortcuts",
    or: "or",
    description: "Available everywhere in Symphony. Typing in a text field is never intercepted.",
    groups: {
      general: "General",
      layout: "Layout"
    },
    items: {
      palette: "Command palette and search",
      newTask: "New task",
      projects: "Switch project",
      refresh: "Refresh data",
      sidebar: "Show or hide the sidebar",
      activity: "Show or hide the run details panel (in a task thread)",
      help: "This help"
    }
  },
  sv: {
    title: "Kortkommandon",
    or: "eller",
    description: "Fungerar överallt i Symphony. Text du skriver i ett fält fångas aldrig upp.",
    groups: {
      general: "Allmänt",
      layout: "Layout"
    },
    items: {
      palette: "Kommandopalett och sök",
      newTask: "Ny uppgift",
      projects: "Byt projekt",
      refresh: "Uppdatera data",
      sidebar: "Visa eller dölj sidofältet",
      activity: "Visa eller dölj körningsdetaljerna (i en uppgiftstråd)",
      help: "Den här hjälpen"
    }
  }
});
