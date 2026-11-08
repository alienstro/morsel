import { act, fireEvent, render, screen } from "@testing-library/react";
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
      </Routes>
    </MemoryRouter>,
  );
}

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
