import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emit, resetTauri, responses } from "../test/tauri";
import { AppRoutes } from "./routes";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../test/tauri")).listen }));

const meeting = (status: string) => ({
  id: 5, title: "Standup", templateId: 1, templateName: "General", status, startedAt: Date.now(), endedAt: null, people: [],
});

beforeEach(() => {
  resetTauri();
  responses.get_settings = { onboarded: true };
  responses.list_templates = [];
  responses.list_meetings = [];
  responses.search_notes = { items: [], total: 0 };
  responses.upcoming_events = [];
  responses.get_meeting = () => new Promise(() => {});
  responses.list_people = [];
  responses.list_asks = [];
});

function renderAt(path: string) {
  render(<MemoryRouter initialEntries={[path]}><AppRoutes /></MemoryRouter>);
}

describe("New note button", () => {
  it.each(["/", "/notes", "/templates", "/ask"])("new_note_on_%s_focuses_the_title", async (path) => {
    renderAt(path);
    await userEvent.click(await screen.findByRole("button", { name: "New note" }));
    await waitFor(() => expect(screen.getByPlaceholderText("Meeting title (optional)")).toHaveFocus());
  });

  it("new_note_twice_on_home_focuses_the_title_again", async () => {
    renderAt("/");
    const button = await screen.findByRole("button", { name: "New note" });
    await userEvent.click(button);
    await waitFor(() => expect(screen.getByPlaceholderText("Meeting title (optional)")).toHaveFocus());
    await userEvent.click(button);
    await waitFor(() => expect(screen.getByPlaceholderText("Meeting title (optional)")).toHaveFocus());
  });

  it("new_note_with_a_calendar_event_focuses_start_notes", async () => {
    responses.upcoming_events = [{ id: "e1", provider: "google", title: "Sync", start: Date.now() + 600_000, end: Date.now() + 1_800_000, link: null, people: [] }];
    renderAt("/notes");
    await userEvent.click(await screen.findByRole("button", { name: "New note" }));
    await screen.findByText("Sync");
    await waitFor(() => expect(screen.getByRole("button", { name: /Start notes/ })).toHaveFocus());
  });

  it("calendar_change_after_new_note_keeps_focus_in_the_ask_box", async () => {
    renderAt("/");
    await userEvent.click(await screen.findByRole("button", { name: "New note" }));
    await waitFor(() => expect(screen.getByPlaceholderText("Meeting title (optional)")).toHaveFocus());
    const askBox = screen.getByPlaceholderText("Ask about any meeting");
    await userEvent.click(askBox);
    responses.upcoming_events = [{ id: "e1", provider: "google", title: "Sync", start: Date.now() + 600_000, end: Date.now() + 1_800_000, link: null, people: [] }];
    act(() => emit("calendar://changed", null));
    await screen.findByText("Sync");
    expect(askBox).toHaveFocus();
  });

  it("new_note_while_recording_opens_that_meeting", async () => {
    responses.list_meetings = [meeting("recording")];
    renderAt("/notes");
    await userEvent.click(await screen.findByRole("button", { name: "New note" }));
    expect(await screen.findByText("Loading the meeting…")).toBeInTheDocument();
  });

  it("new_note_with_the_keyboard_focuses_the_title", async () => {
    renderAt("/notes");
    const button = await screen.findByRole("button", { name: "New note" });
    button.focus();
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByPlaceholderText("Meeting title (optional)")).toHaveFocus());
  });
});
