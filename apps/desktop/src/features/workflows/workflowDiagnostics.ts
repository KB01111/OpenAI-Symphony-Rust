/**
 * Maps the runtime's workflow validation errors (the exact text produced by `symphony-core`'s
 * `WorkflowDefinition::parse`, `ServiceConfig::from_workflow` and the Liquid renderer) to
 * editor diagnostics with document line/column positions.
 *
 * Positions reported by the runtime are relative to the part of the document that failed:
 * YAML errors count from the first front-matter line, Liquid errors from the first line of the
 * trimmed Markdown body. The layout helper below mirrors the runtime's split so those positions
 * can be translated back to lines in the full `WORKFLOW.md` text. Pure functions, no DOM.
 */
import type { WorkflowValidation } from "@symphony/shared-types";

export type KnownWorkflowError =
  | { key: "unclosedFrontMatter" }
  | { key: "frontMatterNotMap" }
  | { key: "unknownFilter"; name: string };

export interface WorkflowDiagnostic {
  severity: "error" | "warning";
  /** Runtime error code (`workflow_parse_error`, `template_render_error`, …). */
  code: string | null;
  /** Concise, single-line message derived from the runtime text. */
  message: string;
  /** Full runtime message (may contain a multi-line Liquid excerpt). */
  raw: string;
  /** Recognised error the UI can show in the active language. */
  known?: KnownWorkflowError;
  /** 1-based document line, when the error could be located. */
  line: number | null;
  /** 1-based column on `line`. */
  column: number | null;
  /** Characters to underline from `column`; `null` underlines to the end of the line. */
  length: number | null;
}

export interface WorkflowSourceLayout {
  /** Text with CRLF normalised to LF (what the runtime parses). */
  normalized: string;
  lines: string[];
  hasFrontMatter: boolean;
  /** 1-based line of the first YAML line (after the opening `---`). */
  frontMatterStartLine: number | null;
  /** 1-based line of the closing `---`, when present. */
  frontMatterEndLine: number | null;
  /** Character offset (in `normalized`) where the trimmed prompt body starts. */
  bodyStartOffset: number;
  /** 1-based line/column where the trimmed prompt body starts. */
  bodyStartLine: number;
  bodyStartColumn: number;
}

/** Mirror of `WorkflowDefinition::parse`'s split between YAML front matter and prompt body. */
export function analyzeWorkflowSource(raw: string): WorkflowSourceLayout {
  const normalized = raw.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  let hasFrontMatter = false;
  let frontMatterStartLine: number | null = null;
  let frontMatterEndLine: number | null = null;
  let bodyOffset = 0;

  if (normalized.startsWith("---\n")) {
    hasFrontMatter = true;
    frontMatterStartLine = 2;
    const rest = normalized.slice(4);
    const end = rest.indexOf("\n---\n");
    if (end >= 0) {
      frontMatterEndLine = 2 + rest.slice(0, end).split("\n").length;
      bodyOffset = 4 + end + 5;
    } else if (rest.endsWith("\n---")) {
      frontMatterEndLine = lines.length;
      bodyOffset = normalized.length;
    } else {
      bodyOffset = normalized.length;
    }
  }

  let start = bodyOffset;
  while (start < normalized.length && /\s/.test(normalized[start]!)) start += 1;
  const pos = offsetToPosition(normalized, start);
  return {
    normalized,
    lines,
    hasFrontMatter,
    frontMatterStartLine,
    frontMatterEndLine,
    bodyStartOffset: start,
    bodyStartLine: pos.line,
    bodyStartColumn: pos.column
  };
}

/** 1-based line/column of a character offset. */
export function offsetToPosition(text: string, offset: number): { line: number; column: number } {
  let line = 1;
  let lineStart = 0;
  const end = Math.min(Math.max(offset, 0), text.length);
  for (let i = 0; i < end; i++) {
    if (text.charCodeAt(i) === 10) {
      line += 1;
      lineStart = i + 1;
    }
  }
  return { line, column: end - lineStart + 1 };
}

const CODE_PREFIX = /^([a-z_]+):\s*/;

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Single-line summary of a runtime error (drops the code prefix and Liquid source excerpts). */
export function summarizeWorkflowError(error: string, code: string | null): string {
  let text = error.trim();
  if (code && text.startsWith(`${code}:`)) text = text.slice(code.length + 1).trim();
  else if (code && text === code) return code.replace(/_/g, " ");
  if (text.startsWith("liquid:")) text = text.slice("liquid:".length).trim();
  const pointer = /^\s*=\s*(.+)$/m.exec(text);
  if (pointer) return pointer[1]!.trim();
  const first = text.split("\n").find((l) => l.trim().length > 0 && !/^-->/.test(l.trim()));
  return (first ?? text).trim();
}

interface Location {
  line: number;
  column: number;
  length: number | null;
}

function searchBody(layout: WorkflowSourceLayout, pattern: RegExp, group = 0): Location | null {
  const re = new RegExp(pattern.source, pattern.flags.includes("g") ? pattern.flags : `${pattern.flags}g`);
  re.lastIndex = layout.bodyStartOffset;
  const match = re.exec(layout.normalized);
  if (!match) return null;
  const text = match[group] ?? match[0];
  const offset = match.index + (group === 0 ? 0 : match[0].indexOf(text));
  const pos = offsetToPosition(layout.normalized, offset);
  return { line: pos.line, column: pos.column, length: text.length };
}

/** Find a dotted YAML key path (`tracker.provider`) inside the front matter. Returns the deepest match. */
export function locateYamlKey(layout: WorkflowSourceLayout, path: string[]): Location | null {
  if (!layout.hasFrontMatter || layout.frontMatterStartLine === null) return null;
  const startIdx = layout.frontMatterStartLine - 1;
  const endIdx = (layout.frontMatterEndLine ?? layout.lines.length + 1) - 1;
  let found: (Location & { indent: number }) | null = null;
  let from = startIdx;
  for (const segment of path) {
    let hit: (Location & { indent: number }) | null = null;
    for (let i = from; i < endIdx; i++) {
      const match = /^(\s*)(?:-\s+)?(["']?)([^\s:#"'][^:#"']*?)\2\s*:(?:\s|$)/.exec(layout.lines[i]!);
      if (!match) continue;
      const indent = match[1]!.length;
      if (found && indent <= found.indent) break;
      if (!found && indent !== 0) continue;
      if (match[3] === segment) {
        hit = { line: i + 1, column: indent + 1 + match[2]!.length, length: segment.length, indent };
        break;
      }
    }
    if (!hit) break;
    found = hit;
    from = hit.line;
  }
  return found ? { line: found.line, column: found.column, length: found.length } : null;
}

function locate(layout: WorkflowSourceLayout, code: string | null, message: string): { loc: Location | null; known?: KnownWorkflowError } {
  switch (code) {
    case "workflow_parse_error": {
      if (/unclosed YAML front matter/i.test(message)) {
        return { loc: { line: 1, column: 1, length: 3 }, known: { key: "unclosedFrontMatter" } };
      }
      const m = /at line (\d+) column (\d+)/.exec(message);
      if (m && layout.frontMatterStartLine !== null) {
        return {
          loc: { line: layout.frontMatterStartLine + Number(m[1]) - 1, column: Number(m[2]), length: null }
        };
      }
      return { loc: layout.hasFrontMatter ? { line: 1, column: 1, length: 3 } : null };
    }
    case "workflow_front_matter_not_a_map":
      return {
        loc: { line: layout.frontMatterStartLine ?? 1, column: 1, length: null },
        known: { key: "frontMatterNotMap" }
      };
    case "template_parse_error":
    case "template_render_error": {
      const filter = /Unknown filter[\s\S]*?requested filter=([^\s,]+)/.exec(message);
      if (filter) {
        const name = filter[1]!;
        return {
          loc: searchBody(layout, new RegExp(`\\|\\s*(${escapeRegExp(name)})\\b`), 1),
          known: { key: "unknownFilter", name }
        };
      }
      const pest = /-->\s*(\d+):(\d+)/.exec(message);
      if (pest) {
        const relLine = Number(pest[1]);
        const relCol = Number(pest[2]);
        return {
          loc: {
            line: layout.bodyStartLine + relLine - 1,
            column: relLine === 1 ? layout.bodyStartColumn + relCol - 1 : relCol,
            length: 1
          }
        };
      }
      const field = /unknown issue field '([^']+)'/.exec(message);
      if (field) return { loc: searchBody(layout, new RegExp(`${escapeRegExp(field[1]!)}\\b`)) };
      const variable = /unknown template variable '([^']+)'/.exec(message);
      if (variable) return { loc: searchBody(layout, new RegExp(`\\{\\{-?\\s*(${escapeRegExp(variable[1]!)})\\b`), 1) };
      return { loc: null };
    }
    case "invalid_config": {
      const keyPath = /\b([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)+)\b/.exec(message);
      if (keyPath) return { loc: locateYamlKey(layout, keyPath[1]!.split(".")) };
      return { loc: null };
    }
    default:
      return { loc: null };
  }
}

/** Convert a runtime validation result into located diagnostics for `raw`. */
export function diagnosticsFromValidation(
  raw: string,
  validation: Pick<WorkflowValidation, "valid" | "error" | "error_code"> | null | undefined
): WorkflowDiagnostic[] {
  if (!validation || validation.valid || !validation.error) return [];
  const code = validation.error_code ?? CODE_PREFIX.exec(validation.error)?.[1] ?? null;
  const layout = analyzeWorkflowSource(raw);
  const { loc, known } = locate(layout, code, validation.error);
  const clamped = loc ? clampLocation(layout, loc) : null;
  return [
    {
      severity: "error",
      code,
      message: summarizeWorkflowError(validation.error, code),
      raw: validation.error,
      known,
      line: clamped?.line ?? null,
      column: clamped?.column ?? null,
      length: clamped?.length ?? null
    }
  ];
}

function clampLocation(layout: WorkflowSourceLayout, loc: Location): Location {
  const line = Math.min(Math.max(loc.line, 1), layout.lines.length);
  const lineLength = layout.lines[line - 1]!.length;
  const column = Math.min(Math.max(loc.column, 1), lineLength + 1);
  return { line, column, length: loc.length };
}

/** Character range `[from, to)` of a diagnostic inside `text` (LF line breaks), for editors. */
export function diagnosticRange(text: string, diagnostic: Pick<WorkflowDiagnostic, "line" | "column" | "length">): { from: number; to: number } | null {
  if (diagnostic.line === null) return null;
  const lines = text.split("\n");
  const lineIdx = Math.min(Math.max(diagnostic.line, 1), lines.length) - 1;
  let lineStart = 0;
  for (let i = 0; i < lineIdx; i++) lineStart += lines[i]!.length + 1;
  const lineLength = lines[lineIdx]!.length;
  const col = Math.min(Math.max(diagnostic.column ?? 1, 1), lineLength + 1) - 1;
  const from = lineStart + col;
  const to = diagnostic.length === null ? lineStart + lineLength : Math.min(from + diagnostic.length, lineStart + lineLength);
  return { from, to: Math.max(from, to) };
}
