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
