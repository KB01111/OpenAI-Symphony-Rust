import React from "react";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import WorkflowStudioPage from "./WorkflowStudioPage.js";
import { diagnosticsFromValidation } from "./workflowDiagnostics.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";

// CodeMirror needs layout APIs jsdom lacks; the page logic is tested with a plain textarea.
vi.mock("./WorkflowEditor.js", () => ({
  default: React.forwardRef<unknown, { value: string; onChange: (v: string) => void; label: string }>(function MockEditor(props, _ref) {
    return <textarea aria-label={props.label} value={props.value} onChange={(e) => props.onChange(e.target.value)} />;
  })
}));

const VALID = "---\ntracker:\n  kind: manual\n---\nWork on {{ issue.identifier }}";
const INVALID = "---\ntracker: [\n---\nWork";

function runtime() {
  let saved = VALID;
  return createMockRuntime({
    "GET /workflow": () => ({ path: "/data/WORKFLOW.md", raw_markdown: saved, valid: true, error: null, config: null, rendered_preview: null }),
    "POST /workflow/preview": ({ body }) => {
      const raw = (body as { raw_markdown: string }).raw_markdown;
      return raw.includes("[")
        ? { valid: false, error_code: "workflow_parse_error", error: "workflow_parse_error: did not find expected node content at line 1 column 10", config: null, rendered_prompt: null, path: "/data/WORKFLOW.md" }
        : { valid: true, error_code: null, error: null, config: null, rendered_prompt: `Work on SAMPLE-1 (${raw.length})`, path: "/data/WORKFLOW.md" };
    },
    "PUT /workflow": ({ body }) => {
      saved = (body as { raw_markdown: string }).raw_markdown;
      return { valid: true, error_code: null, error: null, config: null, rendered_prompt: null, path: "/data/WORKFLOW.md" };
    }
  });
}

describe("WorkflowStudioPage", () => {
  it("validates edits with the runtime and blocks saving invalid workflows", async () => {
    const rt = runtime();
    renderWithProviders(<WorkflowStudioPage />, { runtime: rt });
    const editor = await screen.findByLabelText("WORKFLOW.md editor");
    expect(await screen.findByText("Workflow is valid")).toBeInTheDocument();
    expect(screen.getByText("/data/WORKFLOW.md")).toBeInTheDocument();

    fireEvent.change(editor, { target: { value: INVALID } });
    expect(await screen.findByText("Workflow has errors", {}, { timeout: 3000 })).toBeInTheDocument();
    expect(screen.getByText("Line 2, column 10")).toBeInTheDocument();
    const save = screen.getByRole("button", { name: /Save/ });
    expect(save).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(save);
    expect(rt.calls.some((c) => c.method === "PUT")).toBe(false);
  });

  it("saves the edited document exactly once and becomes clean", async () => {
    const rt = runtime();
    renderWithProviders(<WorkflowStudioPage />, { runtime: rt });
    const editor = await screen.findByLabelText("WORKFLOW.md editor");
    const edited = `${VALID}\n\nAlso run the tests.`;
    fireEvent.change(editor, { target: { value: edited } });
    expect(screen.getByText("Unsaved changes")).toBeInTheDocument();
    const save = screen.getByRole("button", { name: /Save/ });
    await waitFor(() => expect(save).not.toHaveAttribute("aria-disabled", "true"), { timeout: 3000 });
    act(() => {
      fireEvent.click(save);
      fireEvent.click(save);
    });
    await waitFor(() => expect(screen.getByText("Saved")).toBeInTheDocument());
    const puts = rt.calls.filter((c) => c.method === "PUT");
    expect(puts).toHaveLength(1);
    expect(puts[0]!.body).toEqual({ raw_markdown: edited });
  });

  it("reverts unsaved edits only after confirmation", async () => {
    renderWithProviders(<WorkflowStudioPage />, { runtime: runtime() });
    const editor = (await screen.findByLabelText("WORKFLOW.md editor")) as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: `${VALID}\nextra` } });
    fireEvent.click(screen.getByRole("button", { name: /^Revert$/ }));
    const dialog = await screen.findByRole("alertdialog");
    fireEvent.click(await within(dialog).findByRole("button", { name: "Revert" }));
    await waitFor(() => expect(editor.value).toBe(VALID));
  });
});

describe("diagnosticsFromValidation", () => {
  it("maps YAML positions from the front matter to document lines", () => {
    const [d] = diagnosticsFromValidation(INVALID, {
      valid: false,
      error_code: "workflow_parse_error",
      error: "workflow_parse_error: did not find expected node content at line 1 column 10"
    });
    expect(d).toMatchObject({ line: 2, column: 10, code: "workflow_parse_error" });
    expect(diagnosticsFromValidation(VALID, { valid: true, error_code: null, error: null })).toEqual([]);
  });

  it("locates unknown Liquid filters in the prompt body", () => {
    const raw = "---\ntracker:\n  kind: manual\n---\nHello {{ issue.title | shout }}";
    const [d] = diagnosticsFromValidation(raw, {
      valid: false,
      error_code: "template_parse_error",
      error: "template_parse_error: liquid: Unknown filter\n  with:\n    requested filter=shout"
    });
    expect(d!.known).toEqual({ key: "unknownFilter", name: "shout" });
    expect(d!.line).toBe(5);
  });
});
