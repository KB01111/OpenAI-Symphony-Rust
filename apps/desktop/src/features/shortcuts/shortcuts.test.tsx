import { act, fireEvent, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { useGlobalShortcuts } from "../../hooks/useGlobalShortcuts.js";
import { useUiStore } from "../../store/uiStore.js";
import { ShortcutsDialog } from "./ShortcutsDialog.js";
import { shortcutsMessages } from "./messages.js";

const Harness: React.FC = () => {
  useGlobalShortcuts();
  return <input aria-label="field" />;
};

beforeEach(() => {
  useUiStore.setState({ shortcutsOpen: false, paletteOpen: false });
});

describe("keyboard shortcut help", () => {
  const renderHarness = () => renderWithProviders(<Harness />, { runtime: createMockRuntime() });

  it("toggles with F1 and Ctrl+/", () => {
    renderHarness();
    fireEvent.keyDown(window, { key: "F1" });
    expect(useUiStore.getState().shortcutsOpen).toBe(true);
    fireEvent.keyDown(window, { key: "F1" });
    expect(useUiStore.getState().shortcutsOpen).toBe(false);
    fireEvent.keyDown(window, { key: "/", ctrlKey: true });
    expect(useUiStore.getState().shortcutsOpen).toBe(true);
  });

  it("toggles once per key press, not on every auto-repeat keydown", () => {
    renderHarness();
    fireEvent.keyDown(window, { key: "F1" });
    fireEvent.keyDown(window, { key: "F1", repeat: true });
    fireEvent.keyDown(window, { key: "F1", repeat: true });
    expect(useUiStore.getState().shortcutsOpen).toBe(true);
    fireEvent.keyDown(window, { key: "/", ctrlKey: true, repeat: true });
    expect(useUiStore.getState().shortcutsOpen).toBe(true);
  });

  it("is reachable from a text field, but modified F1 is left to the OS", () => {
    renderHarness();
    fireEvent.keyDown(screen.getByLabelText("field"), { key: "/", ctrlKey: true });
    expect(useUiStore.getState().shortcutsOpen).toBe(true);
    act(() => useUiStore.getState().setShortcutsOpen(false));
    fireEvent.keyDown(window, { key: "F1", shiftKey: true });
    fireEvent.keyDown(window, { key: "F1", altKey: true });
    expect(useUiStore.getState().shortcutsOpen).toBe(false);
  });
});

describe("ShortcutsDialog", () => {
  it("lists every shortcut handled by the shell, in the active language", async () => {
    const m = shortcutsMessages.en;
    act(() => useUiStore.setState({ shortcutsOpen: true, locale: "en" }));
    renderWithProviders(<ShortcutsDialog />, { runtime: createMockRuntime() });
    expect(await screen.findByRole("dialog", { name: m.title })).toBeInTheDocument();
    for (const label of Object.values(m.items)) expect(screen.getByText(label)).toBeInTheDocument();
    for (const key of ["K", "N", "P", "R", "B", "J", "I", "F1", "/"]) expect(screen.getAllByText(key).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(useUiStore.getState().shortcutsOpen).toBe(false);
  });

  it("has a Swedish translation for every entry", () => {
    const { en, sv } = shortcutsMessages;
    expect(Object.keys(sv.items)).toEqual(Object.keys(en.items));
    for (const key of Object.keys(en.items) as (keyof typeof en.items)[]) {
      expect(sv.items[key]).not.toBe(en.items[key]);
    }
  });
});
