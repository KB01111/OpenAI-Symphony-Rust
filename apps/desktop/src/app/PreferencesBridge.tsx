import { useEffect, useRef } from "react";
import { isLocale, type Locale } from "@symphony/shared-types";
import { useSaveSettings, useSettings } from "../lib/queries.js";
import type { ThemeMode } from "@symphony/ui";
import { useUiStore } from "../store/uiStore.js";

type Prefs = { theme: string; locale: string };

/**
 * Two-way sync of theme/locale between the local store and runtime settings, tracking which side
 * changed:
 * - a change that arrives from the runtime (startup, or another save) is adopted by the UI;
 * - a change made in the UI (command bar, palette, settings) is written to the runtime as a
 *   functional update on the latest settings, serialized with other settings writes.
 * A failed write is not retried automatically (the error is toasted); the next change retries.
 */
export function PreferencesBridge(): null {
  const settings = useSettings();
  const { mutate } = useSaveSettings();
  const themeMode = useUiStore((s) => s.themeMode);
  const locale = useUiStore((s) => s.locale);
  const setThemeMode = useUiStore((s) => s.setThemeMode);
  const setLocale = useUiStore((s) => s.setLocale);
  // Last values both sides agreed on; `null` until runtime settings have loaded.
  const synced = useRef<Prefs | null>(null);

  const serverTheme = settings.data?.theme;
  const serverLocale = settings.data?.locale;

  // Runtime → UI
  useEffect(() => {
    if (serverTheme === undefined || serverLocale === undefined) return;
    const last = synced.current;
    if (last && last.theme === serverTheme && last.locale === serverLocale) return;
    synced.current = { theme: serverTheme, locale: serverLocale };
    if (serverTheme === "dark" || serverTheme === "light" || serverTheme === "system") setThemeMode(serverTheme);
    if (isLocale(serverLocale)) setLocale(serverLocale);
  }, [serverTheme, serverLocale, setThemeMode, setLocale]);

  // UI → runtime (runs only when the local store changes)
  useEffect(() => {
    const last = synced.current;
    if (!last || (last.theme === themeMode && last.locale === locale)) return;
    synced.current = { theme: themeMode, locale };
    const theme: ThemeMode = themeMode;
    const nextLocale: Locale = locale;
    mutate((latest) => ({ ...latest, theme, locale: nextLocale }));
  }, [themeMode, locale, mutate]);

  return null;
}
