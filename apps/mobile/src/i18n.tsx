import React, { createContext, useContext } from "react";
import { getTranslations, type Locale } from "@symphony/shared-types";
import { mobileMessages, type MobileMessages } from "./messages.js";

const LocaleContext = createContext<Locale>("en");

export const LocaleProvider: React.FC<{ locale: Locale; children: React.ReactNode }> = ({ locale, children }) => (
  <LocaleContext.Provider value={locale}>{children}</LocaleContext.Provider>
);

export function useLocale(): Locale {
  return useContext(LocaleContext);
}

/** Companion strings plus the shared catalogue for the active locale. */
export function useStrings(): { m: MobileMessages; t: ReturnType<typeof getTranslations> } {
  const locale = useLocale();
  return { m: mobileMessages[locale], t: getTranslations(locale) };
}
