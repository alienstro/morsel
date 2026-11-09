import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { formatWhen, meetingLength } from "../../lib/dates";
import { invoke, resetTauri, responses } from "../../test/tauri";
import { NotesScreen } from "./NotesScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));

const NOW = new Date(2026, 10, 9, 12, 0).getTime();

function meeting(id: number, title: string, extra: Record<string, unknown> = {}) {
  return {
    id, title, templateId: 5, templateName: "Product sync", status: "done",
    startedAt: new Date(2026, 10, 9, 10, 0).getTime(), endedAt: new Date(2026, 10, 9, 10, 32).getTime(),
    people: ["Sam", "Priya"], ...extra,
  };
}

function searchCalls() {
  return invoke.mock.calls.filter(([cmd]) => cmd === "search_notes").map(([, args]) => (args as { filter: Record<string, unknown> }).filter);
}

function lastSearch() {
  const calls = searchCalls();
  return calls[calls.length - 1];
}

beforeEach(() => {
  resetTauri();
  responses.search_notes = { items: [meeting(9, "Weekly product sync"), meeting(4, "1:1 with Sam", { people: [], endedAt: null })], total: 48 };
  responses.list_templates = [
    { id: 1, name: "General", description: "", sections: [], instructions: "", builtIn: true, uses: 0 },
    { id: 5, name: "Product sync", description: "", sections: [], instructions: "", builtIn: true, uses: 3 },
  ];
  responses.list_people = ["Priya", "Sam"];
});

function renderNotes() {
  render(
    <MemoryRouter initialEntries={["/notes"]}>
      <Routes>
        <Route path="/notes" element={<NotesScreen />} />
        <Route path="/note/:id" element={<p>note page</p>} />
        <Route path="/meeting/:id" element={<p>meeting page</p>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("NotesScreen", () => {
  it("lists_notes_with_count", async () => {
    renderNotes();
    expect(await screen.findByText("Weekly product sync")).toBeInTheDocument();
    expect(screen.getByText("1:1 with Sam")).toBeInTheDocument();
    expect(screen.getByText("2 of 48 notes")).toBeInTheDocument();
    expect(screen.getByText("Sam, Priya")).toBeInTheDocument();
    expect(screen.getByText("32 min")).toBeInTheDocument();
    expect(screen.getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["Meeting", "Template", "People", "Date", "Length"]);
  });

  it("search_waits_then_sends_query", async () => {
    renderNotes();
    await screen.findByText("Weekly product sync");
    fireEvent.change(screen.getByLabelText("Search notes"), { target: { value: "acme" } });
    expect(searchCalls().some((f) => f.query === "acme")).toBe(false);
    await waitFor(() => expect(searchCalls().some((f) => f.query === "acme" && f.offset === 0)).toBe(true));
  });

  it("template_and_person_filters_send_ids", async () => {
    renderNotes();
    await screen.findByText("Weekly product sync");
    fireEvent.change(screen.getByLabelText("Template"), { target: { value: "5" } });
    await waitFor(() => expect(lastSearch()?.templateId).toBe(5));
    fireEvent.change(screen.getByLabelText("Person"), { target: { value: "Sam" } });
    await waitFor(() => expect(lastSearch()).toMatchObject({ templateId: 5, person: "Sam" }));
  });

  it("row_opens_the_note", async () => {
    responses.search_notes = { items: [meeting(9, "Weekly product sync"), meeting(3, "Live now", { status: "recording" })], total: 2 };
    renderNotes();
    fireEvent.click(await screen.findByRole("link", { name: "Live now" }));
    expect(await screen.findByText("meeting page")).toBeInTheDocument();
  });

  it("row_of_a_finished_meeting_opens_the_note_page", async () => {
    renderNotes();
    fireEvent.click(await screen.findByRole("link", { name: "Weekly product sync" }));
    expect(await screen.findByText("note page")).toBeInTheDocument();
  });

  it("empty_state_shows_a_hint", async () => {
    responses.search_notes = { items: [], total: 0 };
    renderNotes();
    expect(await screen.findByText("Your notes show here after your first meeting.")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Search notes"), { target: { value: "zebra" } });
    expect(await screen.findByText("No note matches your search.")).toBeInTheDocument();
  });

  it("show_more_loads_the_next_page", async () => {
    responses.search_notes = (args?: unknown) => {
      const { filter } = args as { filter: { offset: number } };
      return filter.offset === 0
        ? { items: [meeting(9, "First page")], total: 2 }
        : { items: [meeting(8, "Second page")], total: 2 };
    };
    renderNotes();
    fireEvent.click(await screen.findByRole("button", { name: "Show more" }));
    expect(await screen.findByText("Second page")).toBeInTheDocument();
    expect(screen.getByText("First page")).toBeInTheDocument();
    expect(screen.getByText("2 of 2 notes")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Show more" })).not.toBeInTheDocument();
  });
});

describe("dates", () => {
  it("formats_today_yesterday_and_older_days", () => {
    expect(formatWhen(new Date(2026, 10, 9, 10, 0).getTime(), NOW)).toBe("Today, 10:00");
    expect(formatWhen(new Date(2026, 10, 8, 16, 30).getTime(), NOW)).toBe("Yesterday, 16:30");
    expect(formatWhen(new Date(2026, 10, 2, 9, 30).getTime(), NOW)).toBe("Mon 2 Nov, 09:30");
  });

  it("formats_length_in_minutes", () => {
    expect(meetingLength(0, 32 * 60_000)).toBe("32 min");
    expect(meetingLength(0, null)).toBe("—");
    expect(meetingLength(0, 20_000)).toBe("1 min");
  });
});
