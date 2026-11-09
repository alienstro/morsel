import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { resetTauri, responses } from "../test/tauri";
import { AppRoutes } from "./routes";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../test/tauri")).listen }));

const base = {
  notesBaseUrl: "https://openrouter.ai/api/v1", notesModel: "", quickModel: "", transcribeMode: "local", transcribeTiming: "live",
  transcribeBaseUrl: "", transcribeModel: "", language: "auto", remindBeforeMeeting: true,
  globalShortcut: "CommandOrControl+Shift+M",
};

beforeEach(() => {
  resetTauri();
  responses.list_templates = [];
  responses.list_meetings = [];
  responses.search_notes = { items: [], total: 0 };
  responses.microphone_access = "unknown";
});

describe("onboarding gate", () => {
  it("redirects_to_welcome_when_not_onboarded", async () => {
    responses.get_settings = { ...base, onboarded: false };
    render(<MemoryRouter initialEntries={["/"]}><AppRoutes /></MemoryRouter>);
    expect(await screen.findByText("Step 1 of 3")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Welcome to Morsel" })).toBeInTheDocument();
  });

  it("shows_home_when_onboarded", async () => {
    responses.get_settings = { ...base, onboarded: true };
    render(<MemoryRouter initialEntries={["/"]}><AppRoutes /></MemoryRouter>);
    expect(await screen.findByRole("button", { name: /Start notes/ })).toBeInTheDocument();
    expect(screen.queryByText("Step 1 of 3")).not.toBeInTheDocument();
  });
});
