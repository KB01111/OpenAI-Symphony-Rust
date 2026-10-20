import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { EditorView, basicSetup } from "codemirror";
import { EditorState } from "@codemirror/state";
import { keymap } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { lintGutter, setDiagnostics, type Diagnostic } from "@codemirror/lint";
import { markdown } from "@codemirror/lang-markdown";
import { yamlFrontmatter } from "@codemirror/lang-yaml";
import { tags } from "@lezer/highlight";
import { diagnosticRange, type WorkflowDiagnostic } from "./workflowDiagnostics.js";

export interface WorkflowEditorHandle {
  /** Move the cursor to a 1-based line and focus the editor. */
  focusLine(line: number, column?: number | null): void;
}

export interface WorkflowEditorProps {
  value: string;
  onChange: (value: string) => void;
  onSave: () => void;
  diagnostics: WorkflowDiagnostic[];
  label: string;
}

// Every colour comes from the design tokens, so the editor follows dark/light with the app.
const tokenTheme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "var(--code-background)", color: "var(--foreground)", fontSize: "13px" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.55" },
  ".cm-content": { caretColor: "var(--foreground)" },
  ".cm-gutters": { backgroundColor: "var(--surface-sunken)", color: "var(--subtle-foreground)", borderRight: "1px solid var(--border)" },
  ".cm-activeLine": { backgroundColor: "var(--surface-hover)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--surface-hover)", color: "var(--foreground)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": { backgroundColor: "var(--accent-subtle)" },
  "&.cm-focused": { outline: "none" },
  ".cm-cursor": { borderLeftColor: "var(--foreground)" },
  ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "underline wavy var(--destructive)" },
  ".cm-tooltip": { backgroundColor: "var(--surface-raised)", color: "var(--foreground)", border: "1px solid var(--border)" },
  ".cm-panels": { backgroundColor: "var(--surface)", color: "var(--foreground)" },
  ".cm-searchMatch": { backgroundColor: "var(--warning-subtle)" }
});

const tokenHighlight = HighlightStyle.define([
  { tag: [tags.heading, tags.strong], color: "var(--foreground)", fontWeight: "600" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: [tags.propertyName, tags.definition(tags.propertyName)], color: "var(--accent)" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--success)" },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: "var(--warning)" },
  { tag: [tags.comment, tags.meta, tags.processingInstruction], color: "var(--muted-foreground)" },
  { tag: [tags.link, tags.url], color: "var(--info)", textDecoration: "underline" },
  { tag: [tags.keyword, tags.operator, tags.punctuation, tags.separator], color: "var(--muted-foreground)" }
]);

function toCmDiagnostics(text: string, diagnostics: WorkflowDiagnostic[]): Diagnostic[] {
  return diagnostics.flatMap((d) => {
    const range = diagnosticRange(text, d);
    return range ? [{ from: range.from, to: range.to, severity: d.severity, message: d.message }] : [];
  });
}

/** CodeMirror 6 editor for WORKFLOW.md (Markdown body + YAML front matter), loaded lazily. */
const WorkflowEditor = forwardRef<WorkflowEditorHandle, WorkflowEditorProps>(function WorkflowEditor(
  { value, onChange, onSave, diagnostics, label },
  ref
) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  // Latest callbacks without re-creating the editor.
  const callbacks = useRef({ onChange, onSave });
  callbacks.current = { onChange, onSave };

  useEffect(() => {
    if (!host.current) return;
    const editor = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: value,
        extensions: [
          keymap.of([
            {
              key: "Mod-s",
              preventDefault: true,
              run: () => {
                callbacks.current.onSave();
                return true;
              }
            }
          ]),
          basicSetup,
          yamlFrontmatter({ content: markdown() }),
          syntaxHighlighting(tokenHighlight),
          tokenTheme,
          lintGutter(),
          EditorView.lineWrapping,
          EditorView.contentAttributes.of({ "aria-label": label }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) callbacks.current.onChange(update.state.doc.toString());
          })
        ]
      })
    });
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
    // The editor is created once; external value changes are applied below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Apply external document changes (load, revert, reload from disk) without losing the instance.
  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const current = editor.state.doc.toString();
    if (current !== value) editor.dispatch({ changes: { from: 0, to: current.length, insert: value } });
  }, [value]);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    editor.dispatch(setDiagnostics(editor.state, toCmDiagnostics(editor.state.doc.toString(), diagnostics)));
  }, [diagnostics]);

  useImperativeHandle(ref, () => ({
    focusLine(line, column) {
      const editor = view.current;
      if (!editor) return;
      const doc = editor.state.doc;
      const target = doc.line(Math.min(Math.max(line, 1), doc.lines));
      const pos = Math.min(target.from + Math.max((column ?? 1) - 1, 0), target.to);
      editor.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
      editor.focus();
    }
  }));

  return <div ref={host} className="h-full min-h-0 overflow-hidden" />;
});

export default WorkflowEditor;
