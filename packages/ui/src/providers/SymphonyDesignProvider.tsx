import React, { createContext, useContext, useEffect, useMemo, useSyncExternalStore } from "react";
import { FluentProvider, Toaster } from "@fluentui/react-components";
import { AppsSDKUIProvider } from "@openai/apps-sdk-ui/components/AppsSDKUIProvider";
import { applyDocumentTheme } from "@openai/apps-sdk-ui/theme";
import type { Locale } from "@symphony/shared-types";
import { symphonyDarkTheme, symphonyLightTheme } from "../theme/fluentThemes.js";

export type ThemeMode = "system" | "dark" | "light";
export type ResolvedTheme = "dark" | "light";
export type Density = "compact" | "comfortable";
export type MotionPreference = "system" | "reduced" | "full";

export const SYMPHONY_TOASTER_ID = "symphony-toaster";

export interface DesignContextValue {
  themeMode: ThemeMode;
  resolvedTheme: ResolvedTheme;
  density: Density;
  reducedMotion: boolean;
  locale: Locale;
}

const DesignContext = createContext<DesignContextValue>({
  themeMode: "system",
  resolvedTheme: "dark",
  density: "compact",
  reducedMotion: false,
  locale: "en"
});

export function useDesign(): DesignContextValue {
  return useContext(DesignContext);
}

function mediaQueryStore(query: string, fallback: boolean) {
  return {
    subscribe(callback: () => void) {
      if (typeof window === "undefined" || typeof window.matchMedia !== "function") return () => undefined;
      const mq = window.matchMedia(query);
      mq.addEventListener?.("change", callback);
      return () => mq.removeEventListener?.("change", callback);
    },
    getSnapshot() {
      if (typeof window === "undefined" || typeof window.matchMedia !== "function") return fallback;
      return window.matchMedia(query).matches;
    }
  };
}

const darkQuery = mediaQueryStore("(prefers-color-scheme: dark)", true);
const reducedMotionQuery = mediaQueryStore("(prefers-reduced-motion: reduce)", false);

export interface SymphonyDesignProviderProps {
  themeMode: ThemeMode;
  density?: Density;
  motion?: MotionPreference;
  /** UI locale; also applied to `<html lang>` for screen readers. */
  locale?: Locale;
  children: React.ReactNode;
}

/**
 * Coordinates every visual system from one place:
 * - resolves `system` against the OS (Windows) colour preference and follows live changes;
 * - writes `data-theme`, `data-density`, `data-motion` and `lang` on `<html>`, which drive the
 *   Symphony tokens (Tailwind/shadcn) and Apps SDK UI (`applyDocumentTheme`);
 * - provides the matching Fluent theme, applied to portals so dialogs/menus/tooltips inherit it;
 * - mounts the single app-wide Fluent toaster.
 */
export const SymphonyDesignProvider: React.FC<SymphonyDesignProviderProps> = ({
  themeMode,
  density = "compact",
  motion = "system",
  locale = "en",
  children
}) => {
  const systemDark = useSyncExternalStore(darkQuery.subscribe, darkQuery.getSnapshot, () => true);
  const systemReducedMotion = useSyncExternalStore(
    reducedMotionQuery.subscribe,
    reducedMotionQuery.getSnapshot,
    () => false
  );
  const resolvedTheme: ResolvedTheme = themeMode === "system" ? (systemDark ? "dark" : "light") : themeMode;
  const reducedMotion = motion === "reduced" || (motion === "system" && systemReducedMotion);

  useEffect(() => {
    const html = document.documentElement;
    applyDocumentTheme(resolvedTheme);
    html.dataset.density = density;
    html.dataset.motion = motion === "system" ? (systemReducedMotion ? "reduced" : "system") : motion;
    html.lang = locale;
  }, [resolvedTheme, density, motion, systemReducedMotion, locale]);

  const value = useMemo(
    () => ({ themeMode, resolvedTheme, density, reducedMotion, locale }),
    [themeMode, resolvedTheme, density, reducedMotion, locale]
  );
  const fluentTheme = resolvedTheme === "dark" ? symphonyDarkTheme : symphonyLightTheme;

  return (
    <DesignContext.Provider value={value}>
      <FluentProvider
        theme={fluentTheme}
        applyStylesToPortals
        className="symphony-fluent-root"
        style={{ background: "transparent", color: "inherit", height: "100%" }}
      >
        <AppsSDKUIProvider linkComponent="a">{children}</AppsSDKUIProvider>
        <Toaster toasterId={SYMPHONY_TOASTER_ID} position="bottom-end" pauseOnHover limit={4} />
      </FluentProvider>
    </DesignContext.Provider>
  );
};

