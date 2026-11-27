import { act, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emit, resetTauri, responses } from "../test/tauri";
import { AppRoutes } from "./routes";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../test/tauri")).listen }));

beforeEach(() => {
  resetTauri();
  responses.get_settings = { onboarded: true };
  responses.list_templates = [];
  responses.list_meetings = [];
  responses.search_notes = { items: [], total: 0 };
  responses.get_meeting = () => new Promise(() => {});
  responses.list_people = [];
});

describe("open meeting event", () => {
  it("open_meeting_event_navigates", async () => {
    render(<MemoryRouter initialEntries={["/notes"]}><AppRoutes /></MemoryRouter>);
    await screen.findByLabelText("Search notes");
    act(() => emit("app://open-meeting", { id: 42 }));
    expect(await screen.findByText(/Loading the meeting|Loading/)).toBeInTheDocument();
  });
});

describe("Recent list", () => {
  it("recent_opens_a_live_meeting_on_the_meeting_screen", async () => {
    responses.list_meetings = [
      { id: 100, title: "Live sync", templateId: 1, templateName: "General", status: "recording", startedAt: 0 },
      { id: 101, title: "Paused sync", templateId: 1, templateName: "General", status: "paused", startedAt: 0 },
      { id: 102, title: "Old sync", templateId: 1, templateName: "General", status: "done", startedAt: 0 },
      { id: 103, title: "After sync", templateId: 1, templateName: "General", status: "transcribing", startedAt: 0 },
    ];
    render(<MemoryRouter initialEntries={["/notes"]}><AppRoutes /></MemoryRouter>);
    expect(await screen.findByRole("link", { name: "Live sync" })).toHaveAttribute("href", "/meeting/100");
    expect(screen.getByRole("link", { name: "Paused sync" })).toHaveAttribute("href", "/meeting/101");
    expect(screen.getByRole("link", { name: "Old sync" })).toHaveAttribute("href", "/note/102");
    expect(screen.getByRole("link", { name: "After sync" })).toHaveAttribute("href", "/note/103");
  });
});
