import {
  createDarkTheme,
  createLightTheme,
  type BrandVariants,
  type Theme
} from "@fluentui/react-components";

/** Restrained blue brand ramp used by Fluent for its brand-derived tokens. */
const symphonyBrand: BrandVariants = {
  10: "#04060f",
  20: "#0d1630",
  30: "#122251",
  40: "#162d6c",
  50: "#1a3988",
  60: "#1f45a4",
  70: "#2352c0",
  80: "#2563eb",
  90: "#3f74ee",
  100: "#5b8def",
  110: "#74a0f5",
  120: "#8db2f7",
  130: "#a6c4f9",
  140: "#bfd5fb",
  150: "#d8e6fd",
  160: "#f0f5fe"
};

/**
 * Fluent tokens that describe surfaces, text and strokes point at the Symphony CSS variables in
 * `tokens.css`, so Fluent, shadcn, Tailwind and Apps SDK UI resolve to the same colours and switch
 * together when `data-theme` changes.
 */
const sharedOverrides: Partial<Theme> = {
  fontFamilyBase: "var(--font-sans)",
  fontFamilyMonospace: "var(--font-mono)",
  fontFamilyNumeric: "var(--font-sans)",
  // Body and caption sizes follow the density setting (13/12 px compact, 14/12 px comfortable),
  // matching Tailwind's text-sm/text-xs, so Fluent controls and feature markup line up.
  fontSizeBase300: "var(--density-font)",
  fontSizeBase200: "var(--density-font-xs)",
  colorNeutralBackground1: "var(--surface)",
  colorNeutralBackground1Hover: "var(--surface-hover)",
  colorNeutralBackground1Pressed: "var(--surface-selected)",
  colorNeutralBackground1Selected: "var(--surface-selected)",
  colorNeutralBackground2: "var(--background)",
  colorNeutralBackground2Hover: "var(--surface-hover)",
  colorNeutralBackground3: "var(--surface-sunken)",
  colorNeutralBackground3Hover: "var(--surface-hover)",
  colorNeutralBackground4: "var(--muted)",
  colorSubtleBackgroundHover: "var(--surface-hover)",
  colorSubtleBackgroundPressed: "var(--surface-selected)",
  colorSubtleBackgroundSelected: "var(--surface-selected)",
  colorNeutralForeground1: "var(--foreground)",
  colorNeutralForeground2: "var(--foreground)",
  colorNeutralForeground3: "var(--muted-foreground)",
  colorNeutralForeground4: "var(--subtle-foreground)",
  colorNeutralStroke1: "var(--border-strong)",
  colorNeutralStroke2: "var(--border)",
  colorNeutralStroke3: "var(--border)",
  colorNeutralStrokeAccessible: "var(--border-strong)",
  colorStrokeFocus2: "var(--ring)",
  colorBrandBackground: "var(--accent)",
  colorBrandBackgroundHover: "var(--accent-hover)",
  colorBrandForeground1: "var(--accent)",
  colorBrandStroke1: "var(--accent)",
  colorCompoundBrandStroke: "var(--accent)",
  colorNeutralForegroundOnBrand: "var(--accent-foreground)",
  colorPaletteRedForeground1: "var(--destructive)",
  colorPaletteGreenForeground1: "var(--success)",
  colorStatusDangerForeground1: "var(--destructive)",
  colorStatusWarningForeground1: "var(--warning)",
  colorStatusSuccessForeground1: "var(--success)",
  borderRadiusMedium: "var(--radius)",
  // Flat `var()` references only: Fluent drops token values with nested parentheses such as
  // `calc(var(--radius) + 2px)`, so the arithmetic lives in tokens.css.
  borderRadiusLarge: "var(--radius-lg)",
  borderRadiusXLarge: "var(--radius-xl)"
};

export const symphonyDarkTheme: Theme = { ...createDarkTheme(symphonyBrand), ...sharedOverrides };
export const symphonyLightTheme: Theme = { ...createLightTheme(symphonyBrand), ...sharedOverrides };
