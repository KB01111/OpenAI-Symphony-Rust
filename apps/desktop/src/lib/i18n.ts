import { getTranslations, type Locale, type TranslationDictionary } from "@symphony/shared-types";
import { useUiStore } from "../store/uiStore.js";

export { defineMessages, formatMessage } from "@symphony/shared-types";

export function useLocale(): Locale {
  return useUiStore((s) => s.locale);
}

/** Shared (cross-app) translations for the active locale. */
export function useT(): TranslationDictionary {
  return getTranslations(useLocale());
}

/** Resolve a feature catalogue created with `defineMessages` for the active locale. */
export function useMessages<T>(catalogue: Record<Locale, T>): T {
  return catalogue[useLocale()];
}
