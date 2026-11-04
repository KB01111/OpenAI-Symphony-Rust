import React from "react";
import { FluentProvider } from "@fluentui/react-components";
import { useDesign } from "./providers/SymphonyDesignProvider.js";
import { symphonyDarkTheme, symphonyLightTheme } from "./theme/fluentThemes.js";

/**
 * Fluent UI v9 theme bridge for the mobile companion, which still renders Fluent controls.
 * The desktop app does not use it. Place inside `SymphonyDesignProvider`.
 */
export const FluentBridge: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { resolvedTheme } = useDesign();
  return (
    <FluentProvider
      theme={resolvedTheme === "dark" ? symphonyDarkTheme : symphonyLightTheme}
      applyStylesToPortals
      style={{ background: "transparent", color: "inherit", height: "100%" }}
    >
      {children}
    </FluentProvider>
  );
};

export { symphonyDarkTheme, symphonyLightTheme };
