import { useEffect, useRef } from "react";
import { isLocale } from "@symphony/shared-types";
import { useSaveSettings, useSettings } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";

/**
 * Two-way sync of theme/locale between the local store and runtime settings.
 * - On load, adopt the runtime's persisted values (the local store only makes startup instant).
 * - Afterwards, a theme/language change made anywhere in the UI (command bar, palette, settings)
 *   is written back, so it survives the next launch instead of being reverted on startup.
 */
export function PreferencesBridge(): null {
  const settings = useSettings();
  const save = useSaveSettings();
  const applied = useRef(false);
  const themeMode = useUiStore((s) => s.themeMode);
  const locale = useUiStore((s) => s.locale);
  const setThemeMode = useUiStore((s) => s.setThemeMode);
  const setLocale = useUiStore((s) => s.setLocale);

  const { mutate, isPending } = save;
  useEffect(() => {
    const current = settings.data;
    if (!current) return;
    if (!applied.current) {
      // First load: adopt the runtime values; the store update re-runs this effect with them.
      applied.current = true;
      const { theme, locale: savedLocale } = current;
      if (theme === "dark" || theme === "light" || theme === "system") setThemeMode(theme);
      if (isLocale(savedLocale)) setLocale(savedLocale);
      return;
    }
    if (isPending) return;
    if (current.theme === themeMode && current.locale === locale) return;
    mutate({ ...current, theme: themeMode, locale });
  }, [settings.data, themeMode, locale, setThemeMode, setLocale, mutate, isPending]);

  return null;
}
