import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, resetTauri, responses } from "../../test/tauri";
import { SettingsShortcutsScreen, formatShortcut } from "./SettingsShortcutsScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));

function calls(cmd: string) {
  return invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
}

beforeEach(() => {
  resetTauri();
  responses.get_settings = { globalShortcut: "CommandOrControl+Shift+M" };
  responses.set_global_shortcut = null;
});

describe("SettingsShortcutsScreen", () => {
  it("formats_a_shortcut_with_mac_symbols", () => {
    expect(formatShortcut("CommandOrControl+Shift+M")).toBe("⌘⇧M");
    expect(formatShortcut("Control+Alt+Space")).toBe("⌃⌥Space");
  });

  it("shortcuts_page_saves_a_new_shortcut", async () => {
    render(<MemoryRouter><SettingsShortcutsScreen /></MemoryRouter>);
    expect(await screen.findByText("⌘⇧M")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Change" }));
    expect(screen.getByText("Press the new shortcut. Esc cancels.")).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Shift", code: "ShiftLeft", shiftKey: true, metaKey: true });
    expect(calls("set_global_shortcut")).toHaveLength(0);
    fireEvent.keyDown(window, { key: "k", code: "KeyK", shiftKey: true, metaKey: true });
    await waitFor(() => expect(calls("set_global_shortcut")).toEqual([{ shortcut: "CommandOrControl+Shift+K" }]));
    expect(await screen.findByText("⌘⇧K")).toBeInTheDocument();
  });

  it("refused_shortcut_shows_the_error", async () => {
    responses.set_global_shortcut = () => Promise.reject({ code: "invalid", message: "invalid value: A shortcut needs Command, Control or Option." });
    render(<MemoryRouter><SettingsShortcutsScreen /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: "Change" }));
    fireEvent.keyDown(window, { key: "j", code: "KeyJ", shiftKey: true });
    expect(await screen.findByText("A shortcut needs Command, Control or Option.")).toBeInTheDocument();
    expect(screen.getByText("⌘⇧M")).toBeInTheDocument();
  });

  it("escape_cancels_the_change", async () => {
    render(<MemoryRouter><SettingsShortcutsScreen /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: "Change" }));
    fireEvent.keyDown(window, { key: "Escape", code: "Escape" });
    expect(screen.queryByText("Press the new shortcut. Esc cancels.")).not.toBeInTheDocument();
    expect(calls("set_global_shortcut")).toHaveLength(0);
  });

  it("lists_the_shortcuts_inside_morsel", async () => {
    render(<MemoryRouter><SettingsShortcutsScreen /></MemoryRouter>);
    expect(await screen.findByText("Ask about any meeting")).toBeInTheDocument();
    expect(screen.getByText("⌘K")).toBeInTheDocument();
    expect(screen.getByText("⌘T")).toBeInTheDocument();
  });
});
