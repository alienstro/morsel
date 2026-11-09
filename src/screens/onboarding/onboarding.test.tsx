import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emit, invoke, resetTauri, responses } from "../../test/tauri";
import { OnboardingScreen } from "./OnboardingScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));

const settings = {
  notesBaseUrl: "https://openrouter.ai/api/v1", notesModel: "", quickModel: "", transcribeMode: "local", localModel: "parakeet",
  transcribeBaseUrl: "https://api.groq.com/openai/v1", transcribeModel: "whisper-large-v3-turbo", language: "auto",
  onboarded: false, remindBeforeMeeting: true, globalShortcut: "CommandOrControl+Shift+M",
};

function calls(cmd: string) {
  return invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
}

beforeEach(() => {
  resetTauri();
  responses.get_settings = settings;
  responses.microphone_access = "unknown";
  responses.request_microphone = "allowed";
  responses.model_status = false;
  responses.has_api_key = false;
});

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
    expect(await screen.findByRole("radio", { name: /^On this Mac: Parakeet.*25 European languages, not Tagalog/ })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /^On this Mac: Whisper Turbo.*Tagalog too/ })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /^Cloud.*Tagalog too/ })).toBeInTheDocument();
  });

  it("ai_step_saves_settings_then_tests_the_connection", async () => {
    responses.test_notes_key = null;
    renderAt("/welcome/2");
    expect(await screen.findByText("Step 2 of 3")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "sk-test" } });
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "provider/model" } });
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByText("Connected")).toBeInTheDocument();
    const order = invoke.mock.calls.map(([c]) => c).filter((c) => ["save_settings", "set_api_key", "test_notes_key"].includes(c));
    expect(order).toEqual(["save_settings", "set_api_key", "test_notes_key"]);
    expect(calls("save_settings")[0].settings).toMatchObject({ notesModel: "provider/model" });
    expect(calls("set_api_key")[0]).toEqual({ account: "notes_api_key", key: "sk-test" });
  });

  it("custom_provider_shows_the_base_url", async () => {
    renderAt("/welcome/2");
    await screen.findByText("Step 2 of 3");
    expect(screen.queryByLabelText("Base URL")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "custom" } });
    expect(screen.getByLabelText("Base URL")).toHaveValue("");
  });

  it("ai_step_shows_download_progress", async () => {
    responses.download_model = () => new Promise(() => {});
    renderAt("/welcome/2");
    fireEvent.click(await screen.findByRole("button", { name: "Download the model" }));
    await waitFor(() => expect(calls("download_model")).toEqual([{ model: "parakeet" }]));
    act(() => emit("model://download", { received: 384 * 1_000_000, total: 600 * 1_000_000 }));
    expect(await screen.findByText("Downloading, 384 of 600 MB")).toBeInTheDocument();
  });

  it("ai_step_whisper_choice_downloads_whisper_and_saves_it", async () => {
    responses.download_model = () => new Promise(() => {});
    renderAt("/welcome/2");
    fireEvent.click(await screen.findByRole("radio", { name: /^On this Mac: Whisper Turbo/ }));
    fireEvent.click(screen.getByRole("button", { name: "Download the model" }));
    await waitFor(() => expect(calls("download_model")).toEqual([{ model: "whisper_turbo" }]));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() =>
      expect(calls("save_settings")[0]).toMatchObject({ settings: { transcribeMode: "local", localModel: "whisper_turbo" } }),
    );
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
