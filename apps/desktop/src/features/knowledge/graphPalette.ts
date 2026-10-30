import type { NodeKind } from "@symphony/shared-types";

/** Colours the graph canvas draws with, resolved from the design tokens (never literals). */
export interface GraphPalette {
  byKind: Record<NodeKind, string>;
  changed: string;
  affected: string;
  dim: string;
  edge: string;
  edgeWeak: string;
  label: string;
  /** Behind a hovered label, so it stays readable in both themes. */
  surface: string;
}

/** Token each node kind is drawn with. */
const KIND_TOKEN: Record<NodeKind, string> = {
  package: "--accent",
  module: "--accent",
  file: "--info",
  type: "--warning",
  function: "--success",
  method: "--success",
  dependency: "--muted-foreground",
  other: "--subtle-foreground"
};

/**
 * Resolve a design token to a colour string a WebGL renderer can parse (`rgb(r, g, b)`). A probe
 * element lets the browser do the work, so tokens defined with `var()`, `color-mix()` or any other
 * CSS colour syntax resolve the same way they do in the rest of the UI. If the token is unknown
 * the colour of `host` is used.
 */
export function resolveToken(token: string, host: HTMLElement): string {
  const probe = document.createElement("span");
  probe.style.color = `var(${token})`;
  probe.style.display = "none";
  host.appendChild(probe);
  const resolved = getComputedStyle(probe).color;
  host.removeChild(probe);
  return resolved || getComputedStyle(host).color;
}

/** Read the palette for the current theme from `host` (an element inside the themed document). */
export function readPalette(host: HTMLElement): GraphPalette {
  const token = (name: string) => resolveToken(name, host);
  const byKind = Object.fromEntries(
    (Object.keys(KIND_TOKEN) as NodeKind[]).map((kind) => [kind, token(KIND_TOKEN[kind])])
  ) as Record<NodeKind, string>;
  return {
    byKind,
    changed: token("--warning"),
    affected: token("--accent"),
    dim: token("--border"),
    edge: token("--border-strong"),
    edgeWeak: token("--border"),
    label: token("--foreground"),
    surface: token("--surface-raised")
  };
}
