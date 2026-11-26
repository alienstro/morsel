import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { resetModelDownloads } from "../../lib/localModel";
import { emit, invoke, resetTauri, responses } from "../../test/tauri";
import { SettingsModelsScreen } from "../SettingsModelsScreen";
import { OnboardingScreen } from "./OnboardingScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));

const settings = {
  notesMode: "local", notesBaseUrl: "https://openrouter.ai/api/v1", notesModel: "", quickModel: "", transcribeMode: "local", transcribeTiming: "live",
  transcribeBaseUrl: "https://api.groq.com/openai/v1", transcribeModel: "whisper-large-v3-turbo", language: "auto",
  onboarded: false, remindBeforeMeeting: true, globalShortcut: "CommandOrControl+Shift+M",
};

function calls(cmd: string) {
  return invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
}

beforeEach(() => {
  resetTauri();
  resetModelDownloads();
  responses.get_settings = settings;
  responses.microphone_access = "unknown";
  responses.request_microphone = "allowed";
  responses.model_status = false;
  responses.has_api_key = false;
});

/** Leaves the AI step with Back and comes back with Continue. */
async function backAndContinue() {
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  await screen.findByText("Allowed");
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  await screen.findByText("Step 2 of 3");
}

function renderAt(path: string) {
  render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/welcome/:step" element={<OnboardingScreen />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("Welcome step", () => {
  it("welcome_continue_needs_the_microphone", async () => {
    renderAt("/welcome/1");
    expect(await screen.findByText("Step 1 of 3")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    expect(screen.getByText("Allow the microphone to continue.")).toBeInTheDocument();
  });

  it("welcome_asks_for_the_microphone", async () => {
    renderAt("/welcome/1");
    fireEvent.click(await screen.findByRole("button", { name: "Allow the microphone" }));
    expect(await screen.findByText("Allowed")).toBeInTheDocument();
    expect(calls("request_microphone")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByText("Step 2 of 3")).toBeInTheDocument();
  });

  it("denied_microphone_opens_system_settings", async () => {
    responses.microphone_access = "denied";
    renderAt("/welcome/1");
    fireEvent.click(await screen.findByRole("button", { name: "Open System Settings for the microphone" }));
    expect(calls("open_privacy_settings")).toEqual([{ pane: "microphone" }]);
  });

  it("system_audio_asks_macos_then_opens_system_settings", async () => {
    renderAt("/welcome/1");
    expect(screen.queryByRole("button", { name: "Open System Settings for system audio" })).toBeNull();
    fireEvent.click(await screen.findByRole("button", { name: "Allow system audio" }));
    expect(calls("request_system_audio")).toHaveLength(1);
    fireEvent.click(await screen.findByRole("button", { name: "Open System Settings for system audio" }));
    expect(calls("open_privacy_settings")).toEqual([{ pane: "audio_capture" }]);
  });

  it("failed_system_audio_request_still_offers_system_settings", async () => {
    responses.request_system_audio = () => Promise.reject(new Error("tap failed"));
    renderAt("/welcome/1");
    fireEvent.click(await screen.findByRole("button", { name: "Allow system audio" }));
    expect(await screen.findByRole("button", { name: "Open System Settings for system audio" })).toBeInTheDocument();
  });
});

describe("AI step", () => {
  it("transcription_options_name_their_languages", async () => {
    renderAt("/welcome/2");
    expect(await screen.findByRole("radio", { name: /^On this Mac: Whisper Turbo.*Tagalog too/ })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /^Cloud.*Tagalog too/ })).toBeInTheDocument();
    expect(within(screen.getByRole("radiogroup", { name: "Transcription" })).getAllByRole("radio")).toHaveLength(2);
    expect(screen.queryByText(/Parakeet/)).not.toBeInTheDocument();
  });

  it("ai_step_saves_settings_then_tests_the_connection", async () => {
    responses.test_notes_key = null;
    renderAt("/welcome/2");
    expect(await screen.findByText("Step 2 of 3")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: /^Cloud.*your own key/ }));
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "sk-test" } });
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "provider/model" } });
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByText("Connected")).toBeInTheDocument();
    const order = invoke.mock.calls.map(([c]) => c).filter((c) => ["save_settings", "set_api_key", "test_notes_key"].includes(c));
    expect(order).toEqual(["save_settings", "set_api_key", "test_notes_key"]);
    expect(calls("save_settings")[0].settings).toMatchObject({ notesMode: "cloud", notesModel: "provider/model" });
    expect(calls("set_api_key")[0]).toEqual({ account: "notes_api_key", key: "sk-test" });
  });

  it("custom_provider_shows_the_base_url", async () => {
    renderAt("/welcome/2");
    await screen.findByText("Step 2 of 3");
    fireEvent.click(screen.getByRole("radio", { name: /^Cloud.*your own key/ }));
    expect(screen.queryByLabelText("Base URL")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "custom" } });
    expect(screen.getByLabelText("Base URL")).toHaveValue("");
  });

  it("ai_step_saves_the_after_timing", async () => {
    renderAt("/welcome/2");
    expect(await screen.findByRole("radio", { name: /^While the meeting runs/ })).toBeChecked();
    fireEvent.click(screen.getByRole("radio", { name: /^After the meeting.*about 20 minutes/ }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(calls("save_settings")[0]).toMatchObject({ settings: { transcribeTiming: "after" } }));
  });

  it("ai_step_shows_download_progress", async () => {
    responses.download_model = () => new Promise(() => {});
    renderAt("/welcome/2");
    fireEvent.click(await screen.findByRole("button", { name: "Download the model" }));
    await waitFor(() => expect(calls("download_model")).toHaveLength(1));
    act(() => emit("model://download", { received: 384 * 1_000_000, total: 600 * 1_000_000 }));
    expect(await screen.findByText("Downloading, 384 of 600 MB")).toBeInTheDocument();
  });

  it("ai_step_offers_the_local_notes_model", async () => {
    responses.download_notes_model = () => new Promise(() => {});
    renderAt("/welcome/2");
    expect(await screen.findByRole("radio", { name: /^On this Mac: MiniCPM-V-4.*2\.96 GB/ })).toBeChecked();
    expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Download the models (3.27 GB)" }));
    await waitFor(() => expect(calls("download_notes_model")).toHaveLength(1));
    act(() => emit("notes-model://download", { received: 740 * 1_000_000, total: 2960 * 1_000_000 }));
    expect(await screen.findByText("Downloading, 740 of 2960 MB")).toBeInTheDocument();
  });

  it("ai_step_states_the_total_download_size", async () => {
    let finishNotes = () => {};
    responses.download_notes_model = () => new Promise<void>((resolve) => (finishNotes = resolve));
    responses.download_search_model = () => new Promise(() => {});
    renderAt("/welcome/2");
    expect(await screen.findByRole("radio", { name: /3\.27 GB in all/ })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Download the models (3.27 GB)" }));
    await waitFor(() => expect(calls("download_notes_model")).toHaveLength(1));
    expect(calls("download_search_model")).toHaveLength(0);
    act(() => finishNotes());
    await waitFor(() => expect(calls("download_search_model")).toHaveLength(1));
    act(() => emit("search-model://download", { received: 155 * 1_000_000, total: 310 * 1_000_000 }));
    expect(await screen.findByText("Search model: downloading, 155 of 310 MB")).toBeInTheDocument();
  });

  it("ai_step_keeps_the_download_after_back_and_continue", async () => {
    let finish = () => {};
    responses.microphone_access = "allowed";
    responses.download_model = () => new Promise<void>((resolve) => (finish = resolve));
    renderAt("/welcome/2");
    fireEvent.click(await screen.findByRole("button", { name: "Download the model" }));
    act(() => emit("model://download", { received: 200 * 1_000_000, total: 1000 * 1_000_000 }));
    expect(await screen.findByText("Downloading, 200 of 1000 MB")).toBeInTheDocument();
    await backAndContinue();
    expect(await screen.findByText("Downloading, 200 of 1000 MB")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Download the model" })).toBeNull();
    await act(async () => finish());
    expect(await screen.findByText("Ready")).toBeInTheDocument();
    expect(calls("download_model")).toHaveLength(1);
  });

  it("ai_step_shows_a_download_error_after_coming_back", async () => {
    let fail = (_: unknown) => {};
    responses.microphone_access = "allowed";
    responses.download_model = () => new Promise<void>((_, reject) => (fail = reject));
    renderAt("/welcome/2");
    fireEvent.click(await screen.findByRole("button", { name: "Download the model" }));
    await waitFor(() => expect(calls("download_model")).toHaveLength(1));
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByText("Allowed");
    await act(async () => fail({ code: "network", message: "no network" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("no network");
  });

  it("ai_step_keeps_both_local_downloads_after_back_and_continue", async () => {
    let finishNotes = () => {};
    responses.microphone_access = "allowed";
    responses.download_notes_model = () => new Promise<void>((resolve) => (finishNotes = resolve));
    responses.download_search_model = () => new Promise(() => {});
    renderAt("/welcome/2");
    fireEvent.click(await screen.findByRole("button", { name: "Download the models (3.27 GB)" }));
    act(() => emit("notes-model://download", { received: 740 * 1_000_000, total: 2960 * 1_000_000 }));
    await backAndContinue();
    expect(await screen.findByText("Downloading, 740 of 2960 MB")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByText("Allowed");
    responses.notes_model_status = true;
    await act(async () => finishNotes());
    await waitFor(() => expect(calls("download_search_model")).toHaveLength(1));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByText("Step 2 of 3");
    act(() => emit("search-model://download", { received: 155 * 1_000_000, total: 310 * 1_000_000 }));
    expect(await screen.findByText("Search model: downloading, 155 of 310 MB")).toBeInTheDocument();
    expect(calls("download_notes_model")).toHaveLength(1);
  });

  it("settings_shows_the_download_that_onboarding_started", async () => {
    responses.download_model = () => new Promise(() => {});
    renderAt("/welcome/2");
    fireEvent.click(await screen.findByRole("button", { name: "Download the model" }));
    act(() => emit("model://download", { received: 200 * 1_000_000, total: 1000 * 1_000_000 }));
    cleanup();
    render(<MemoryRouter><SettingsModelsScreen /></MemoryRouter>);
    expect(await screen.findByText("Downloading: 20%")).toBeInTheDocument();
    expect(calls("download_model")).toHaveLength(1);
  });

  it("ai_step_back_goes_to_step_1", async () => {
    renderAt("/welcome/2");
    fireEvent.click(await screen.findByRole("button", { name: "Back" }));
    expect(await screen.findByText("Step 1 of 3")).toBeInTheDocument();
  });

  it("ai_step_continue_saves_and_goes_to_step_3", async () => {
    renderAt("/welcome/2");
    await screen.findByText("Step 2 of 3");
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByText("Step 3 of 3")).toBeInTheDocument();
    expect(calls("save_settings")).toHaveLength(1);
    expect(calls("set_api_key")).toHaveLength(0);
  });
});
