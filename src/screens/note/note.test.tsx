import { act, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { markYours } from "../../lib/markYours";
import { emit, invoke, resetTauri, responses } from "../../test/tauri";
import { NoteScreen } from "./NoteScreen";
import { WritingScreen } from "./WritingScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(async () => "/tmp/x.md") }));

const summary = { id: 1, title: "Weekly product sync", templateId: 5, templateName: "Product sync", status: "writing_notes", startedAt: 0 };

function renderAt(path: string) {
  render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/meeting/:id/writing" element={<WritingScreen />} />
        <Route path="/note/:id" element={<NoteScreen />} />
      </Routes>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  resetTauri();
  responses.list_templates = [{ id: 1, name: "General", sections: [], instructions: "" }, { id: 5, name: "Product sync", sections: [], instructions: "" }];
});

describe("markYours", () => {
  it("markYours_matches_case_and_space_insensitive", () => {
    const parts = markYours("## Summary\n-  Budget OK for Q1 \nFinance signed it off.", "budget ok for q1");
    expect(parts).toEqual([
      { text: "## Summary", yours: false },
      { text: "-  Budget OK for Q1 ", yours: true },
      { text: "Finance signed it off.", yours: false },
    ]);
  });

  it("markYours_ignores_empty_lines", () => {
    const parts = markYours("a\n\nb", "\n\n");
    expect(parts.map((p) => p.yours)).toEqual([false, false, false]);
  });
});

describe("WritingScreen", () => {
  beforeEach(() => {
    responses.get_meeting = { summary, lines: [], userNotes: "budget ok\nask Priya", note: null };
    responses.cancel_notes = true;
  });

  it("writing_screen_cancel_calls_cancelNotes_and_shows_rough_notes", async () => {
    renderAt("/meeting/1/writing");
    fireEvent.click(await screen.findByRole("button", { name: "Cancel and keep my notes" }));
    expect(invoke).toHaveBeenCalledWith("cancel_notes", { meetingId: 1 });
    act(() => emit("meeting://status", { meetingId: 1, status: "notes_cancelled", warning: null }));
    expect(await screen.findByText(/ask Priya/)).toBeInTheDocument();
  });

  it("notes_failed_shows_message_and_try_again", async () => {
    renderAt("/meeting/1/writing");
    await screen.findByRole("button", { name: "Cancel and keep my notes" });
    act(() => emit("notes://failed", { meetingId: 1, message: "the API key is not valid" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("the API key is not valid");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(invoke).toHaveBeenCalledWith("write_notes", { meetingId: 1, templateId: 5 });
  });

  it("notes_done_opens_the_note", async () => {
    responses.get_meeting = { summary: { ...summary, status: "done" }, lines: [], userNotes: "budget ok", note: "## Summary\nbudget ok" };
    renderAt("/meeting/1/writing");
    expect(await screen.findByRole("button", { name: "Export as Markdown" })).toBeInTheDocument();
  });
});

describe("NoteScreen", () => {
  it("note_screen_renders_legend_and_grey_lines", async () => {
    responses.get_meeting = {
      summary: { ...summary, status: "done" },
      lines: [],
      userNotes: "budget ok",
      note: "## Summary\n- budget ok\n- Finance signed it off.\n## Action items\n- [ ] Sam: send the plan",
    };
    renderAt("/note/1");
    expect(await screen.findByRole("heading", { name: "Weekly product sync" })).toBeInTheDocument();
    expect(screen.getByText("Your words")).toBeInTheDocument();
    expect(screen.getByText("Added by Morsel")).toBeInTheDocument();
    expect(screen.getByText("budget ok").closest("li")).not.toHaveClass("morsel");
    expect(screen.getByText("Finance signed it off.").closest("li")).toHaveClass("morsel");
    expect(screen.getByRole("heading", { name: "Action items" })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Sam: send the plan" })).toBeInTheDocument();
  });

  it("export_saves_markdown_through_the_backend", async () => {
    responses.get_meeting = { summary: { ...summary, status: "done" }, lines: [], userNotes: "", note: "## Summary\nok" };
    renderAt("/note/1");
    fireEvent.click(await screen.findByRole("button", { name: "Export as Markdown" }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("export_note", { meetingId: 1, path: "/tmp/x.md" }));
  });
});
