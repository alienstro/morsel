import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emit, invoke, resetTauri, responses } from "../test/tauri";
import { HomeScreen } from "./HomeScreen";
import { SettingsModelsScreen } from "./SettingsModelsScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../test/tauri")).listen }));

const settings = {
  notesBaseUrl: "https://openrouter.ai/api/v1", notesModel: "", quickModel: "", transcribeMode: "local",
  transcribeBaseUrl: "https://api.groq.com/openai/v1", transcribeModel: "whisper-large-v3-turbo", language: "auto",
};

beforeEach(() => {
  resetTauri();
  responses.list_templates = [{ id: 1, name: "General", sections: [], instructions: "" }, { id: 3, name: "Sales call", sections: [], instructions: "" }];
  responses.list_meetings = [{ id: 9, title: "Acme discovery call", templateId: 3, templateName: "Sales call", status: "done", startedAt: 0 }];
  responses.get_settings = settings;
  responses.has_api_key = false;
  responses.model_status = false;
});

function renderAt(path: string) {
  render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/" element={<HomeScreen />} />
        <Route path="/settings/models" element={<SettingsModelsScreen />} />
        <Route path="/meeting/:id" element={<p>meeting page</p>} />
        <Route path="/ask" element={<p>ask page</p>} />
        <Route path="/notes" element={<p>notes page</p>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("HomeScreen earlier notes", () => {
  it("home_groups_notes_by_day", async () => {
    const at = (daysBack: number, h: number) => {
      const d = new Date();
      d.setDate(d.getDate() - daysBack);
      d.setHours(h, 30, 0, 0);
      return d.getTime();
    };
    const row = (id: number, title: string, startedAt: number) => ({ id, title, templateId: 1, templateName: "General", status: "done", startedAt, endedAt: null, people: [] });
    responses.list_meetings = [row(1, "Standup", at(0, 9)), row(2, "1:1 with Sam", at(1, 16)), row(3, "Design review", at(1, 11)), row(4, "Old one", at(20, 10))];
    renderAt("/");
    const today = await screen.findByRole("heading", { name: "Today" });
    expect(today).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Yesterday" })).toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 3 })).toHaveLength(3);
    expect(screen.getByText("16:30")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("link", { name: "View all" }));
    expect(await screen.findByText("notes page")).toBeInTheDocument();
  });
});

describe("HomeScreen ask box", () => {
  it("home_cmd_k_focuses_ask_box", async () => {
    renderAt("/");
    const box = await screen.findByLabelText("Ask about any meeting");
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    expect(box).toHaveFocus();
    fireEvent.change(box, { target: { value: "open action items" } });
    fireEvent.submit(box.closest("form")!);
    expect(await screen.findByText("ask page")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByLabelText("Ask about any meeting")).not.toBeInTheDocument());
  });
});

describe("HomeScreen", () => {
  it("start_notes_uses_the_selected_template", async () => {
    responses.start_meeting = 12;
    renderAt("/");
    expect(await screen.findByText("Acme discovery call")).toBeInTheDocument();
    fireEvent.change(await screen.findByLabelText("Template"), { target: { value: "3" } });
    fireEvent.change(screen.getByLabelText("Meeting title"), { target: { value: "Acme follow-up" } });
    fireEvent.click(screen.getByRole("button", { name: "Start notes" }));
    expect(await screen.findByText("meeting page")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("start_meeting", { title: "Acme follow-up", templateId: 3 });
  });

  it("start_error_is_shown", async () => {
    responses.start_meeting = () => Promise.reject({ code: "mic_permission_denied", message: "Morsel has no microphone permission" });
    renderAt("/");
    fireEvent.click(await screen.findByRole("button", { name: "Start notes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("microphone permission");
  });
});

describe("SettingsModelsScreen", () => {
  it("key_field_never_shows_a_saved_key", async () => {
    responses.has_api_key = true;
    renderAt("/settings/models");
    const field = await screen.findByLabelText("Notes API key");
    expect(field).toHaveValue("");
    expect(field).toHaveAttribute("type", "password");
    expect((await screen.findAllByText("A key is saved.")).length).toBeGreaterThan(0);
  });

  it("test_key_shows_ok_and_error_messages", async () => {
    responses.test_notes_key = undefined;
    renderAt("/settings/models");
    fireEvent.click(await screen.findByRole("button", { name: "Test key" }));
    expect(await screen.findByText("The key works.")).toBeInTheDocument();
    responses.test_notes_key = () => Promise.reject({ code: "invalid_key", message: "the API key is not valid" });
    fireEvent.click(screen.getByRole("button", { name: "Test key" }));
    expect(await screen.findByText("the API key is not valid")).toBeInTheDocument();
  });

  it("download_shows_progress_from_events", async () => {
    responses.download_model = () => new Promise(() => {});
    renderAt("/settings/models");
    fireEvent.click(await screen.findByRole("button", { name: "Download the model" }));
    act(() => emit("model://download", { received: 335_000_000, total: 670_000_000 }));
    expect(await screen.findByText("Downloading: 50%")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("value", "50");
  });

  it("save_settings_sends_the_form", async () => {
    renderAt("/settings/models");
    fireEvent.change(await screen.findByLabelText("Notes model"), { target: { value: "anthropic/model" } });
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("save_settings", { settings: { ...settings, notesModel: "anthropic/model" } }),
    );
  });
});
