import { useEffect, useRef } from "react";
import { isLocale } from "@symphony/shared-types";
import { useSettings } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";

/**
 * Adopt theme/locale from runtime settings once they load. The local store makes startup instant
 * (no flash of the wrong theme); the runtime settings are the persisted source of truth.
 */
export function PreferencesBridge(): null {
  const settings = useSettings();
  const applied = useRef(false);
  const setThemeMode = useUiStore((s) => s.setThemeMode);
  const setLocale = useUiStore((s) => s.setLocale);
  useEffect(() => {
    if (!settings.data || applied.current) return;
    applied.current = true;
    const { theme, locale } = settings.data;
    if (theme === "dark" || theme === "light" || theme === "system") setThemeMode(theme);
    if (isLocale(locale)) setLocale(locale);
  }, [settings.data, setThemeMode, setLocale]);
  return null;
}
