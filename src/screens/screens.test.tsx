import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emit, invoke, resetTauri, responses } from "../test/tauri";
import { HomeScreen } from "./HomeScreen";
import { SettingsModelsScreen } from "./SettingsModelsScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../test/tauri")).listen }));

const settings = {
  notesMode: "cloud", notesBaseUrl: "https://openrouter.ai/api/v1", notesModel: "", quickModel: "", transcribeMode: "local", transcribeTiming: "live",
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

  it("test_key_saves_the_form_values_first", async () => {
    responses.has_api_key = true;
    renderAt("/settings/models");
    fireEvent.change(await screen.findByLabelText(/^Base URL \(OpenRouter/), { target: { value: "https://qx13.invalid/v1" } });
    fireEvent.change(screen.getByLabelText("Notes model"), { target: { value: "qx13/typed-not-saved" } });
    fireEvent.click(screen.getByRole("button", { name: "Test key" }));
    expect(await screen.findByText("The key works.")).toBeInTheDocument();
    expect(screen.getByText("Settings saved.")).toBeInTheDocument();
    const order =invoke.mock.calls.map(([c]) => c).filter((c) => ["save_settings", "test_notes_key"].includes(c));
    expect(order).toEqual(["save_settings", "test_notes_key"]);
    expect(invoke).toHaveBeenCalledWith("save_settings", {
      settings: { ...settings, notesBaseUrl: "https://qx13.invalid/v1", notesModel: "qx13/typed-not-saved" },
    });
  });

  it("test_key_asks_for_a_key_when_none_is_saved", async () => {
    responses.has_api_key = false;
    renderAt("/settings/models");
    fireEvent.click(await screen.findByRole("button", { name: "Test key" }));
    expect(await screen.findByText("Add and save the notes API key first.")).toBeInTheDocument();
    expect(invoke.mock.calls.map(([c]) => c)).not.toContain("test_notes_key");
  });

  it("test_key_button_is_disabled_while_a_test_runs", async () => {
    responses.has_api_key = true;
    responses.test_notes_key = () => new Promise(() => {});
    renderAt("/settings/models");
    fireEvent.click(await screen.findByRole("button", { name: "Test key" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Test key" })).toBeDisabled());
  });

  it("key_result_clears_after_an_edit", async () => {
    responses.has_api_key = true;
    renderAt("/settings/models");
    fireEvent.click(await screen.findByRole("button", { name: "Test key" }));
    expect(await screen.findByText("The key works.")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Notes model"), { target: { value: "provider/other" } });
    expect(screen.queryByText("The key works.")).not.toBeInTheDocument();
  });

  it("settings_saved_message_clears_after_an_edit", async () => {
    renderAt("/settings/models");
    fireEvent.click(await screen.findByRole("button", { name: "Save settings" }));
    expect(await screen.findByText("Settings saved.")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Notes model"), { target: { value: "provider/other" } });
    expect(screen.queryByText("Settings saved.")).not.toBeInTheDocument();
  });

  it("test_key_shows_ok_and_error_messages", async () => {
    responses.has_api_key = true;
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

  it("settings_shows_whisper_as_the_only_local_model", async () => {
    responses.download_model = () => new Promise(() => {});
    renderAt("/settings/models");
    expect(await screen.findByText(/Whisper Turbo model \(about 1 GB\): not downloaded/)).toBeInTheDocument();
    expect(screen.queryByText(/Parakeet/)).not.toBeInTheDocument();
    expect(screen.queryByRole("radio", { name: /^Whisper Turbo/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Download the model" }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("download_model"));
  });

  it("settings_saves_the_after_timing", async () => {
    renderAt("/settings/models");
    expect(await screen.findByRole("radio", { name: /^While the meeting runs/ })).toBeChecked();
    fireEvent.click(screen.getByRole("radio", { name: /^After the meeting/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("save_settings", { settings: { ...settings, transcribeTiming: "after" } }),
    );
  });

  it("notes_mode_switch_saves_local", async () => {
    renderAt("/settings/models");
    fireEvent.click(await screen.findByRole("radio", { name: "On this Mac (private, no key)" }));
    expect(screen.queryByLabelText("Notes API key")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("save_settings", { settings: { ...settings, notesMode: "local" } }),
    );
  });

  it("local_mode_shows_the_download_and_size", async () => {
    responses.get_settings = { ...settings, notesMode: "local" };
    responses.notes_model_status = false;
    responses.download_notes_model = () => new Promise(() => {});
    renderAt("/settings/models");
    expect(await screen.findByText(/MiniCPM-V-4 model \(2\.96 GB\): not downloaded/)).toBeInTheDocument();
    expect(screen.queryByLabelText("Notes API key")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Download the notes model" }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("download_notes_model"));
    act(() => emit("notes-model://download", { received: 740_000_000, total: 2_960_000_000 }));
    expect(await screen.findByText("Downloading: 25%")).toBeInTheDocument();
  });

  it("settings_shows_the_search_model_and_its_download", async () => {
    responses.search_model_status = false;
    responses.download_search_model = () => new Promise(() => {});
    renderAt("/settings/models");
    expect(await screen.findByText(/EmbeddingGemma 2 search model \(310 MB\): not downloaded/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Download the search model" }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("download_search_model"));
    act(() => emit("search-model://download", { received: 155_000_000, total: 310_000_000 }));
    expect(await screen.findByText("Downloading: 50%")).toBeInTheDocument();
  });

  it("settings_shows_the_index_progress", async () => {
    renderAt("/settings/models");
    await screen.findByText(/EmbeddingGemma 2 search model/);
    act(() => emit("search://progress", { done: 3, total: 10 }));
    expect(await screen.findByText("Search index: 3 of 10")).toBeInTheDocument();
    act(() => emit("search://progress", { done: 10, total: 10 }));
    await vi.waitFor(() => expect(screen.queryByText(/Search index:/)).not.toBeInTheDocument());
  });

  it("cloud_mode_shows_the_key_fields", async () => {
    renderAt("/settings/models");
    expect(await screen.findByLabelText("Notes API key")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Cloud (your key)" , checked: true })).toBeInTheDocument();
    expect(screen.queryByText(/MiniCPM-V-4/)).not.toBeInTheDocument();
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

describe("HomeScreen next meeting", () => {
  const inMin = (m: number) => Date.now() + m * 60_000;
  const event = (extra: Record<string, unknown> = {}) => ({
    id: "e1", provider: "google", title: "Weekly product sync", start: inMin(6), end: inMin(36),
    link: "https://meet.google.com/abc", people: ["Sam", "Priya", "Leo"], ...extra,
  });

  it("home_shows_the_next_meeting_card", async () => {
    responses.upcoming_events = [event()];
    renderAt("/");
    expect(await screen.findByText("Weekly product sync")).toBeInTheDocument();
    expect(screen.getByText(/Google Meet with Sam, Priya and Leo, in [56] min/)).toBeInTheDocument();
    expect(screen.queryByPlaceholderText("Meeting title (optional)")).not.toBeInTheDocument();
  });

  it("card_start_sends_title_and_people", async () => {
    responses.upcoming_events = [event()];
    responses.start_meeting = 42;
    renderAt("/");
    await screen.findByText("Weekly product sync");
    fireEvent.change(screen.getByLabelText("Template"), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: /Start notes/ }));
    expect(await screen.findByText("meeting page")).toBeInTheDocument();
    const args = invoke.mock.calls.find(([c]) => c === "start_meeting")?.[1];
    expect(args).toEqual({ title: "Weekly product sync", templateId: 3, people: ["Sam", "Priya", "Leo"] });
  });

  it("a_started_meeting_shows_now", async () => {
    responses.upcoming_events = [event({ start: inMin(-3), link: null, people: [] })];
    renderAt("/");
    expect(await screen.findByText("Now")).toBeInTheDocument();
  });

  it("home_without_calendar_shows_the_plain_start", async () => {
    responses.upcoming_events = [];
    renderAt("/");
    expect(await screen.findByPlaceholderText("Meeting title (optional)")).toBeInTheDocument();
  });

  it("unscheduled_link_shows_the_plain_start", async () => {
    responses.upcoming_events = [event()];
    renderAt("/");
    fireEvent.click(await screen.findByRole("button", { name: "Start a note without the calendar" }));
    expect(screen.getByPlaceholderText("Meeting title (optional)")).toBeInTheDocument();
  });
});
