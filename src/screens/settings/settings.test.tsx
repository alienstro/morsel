import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, resetTauri, responses } from "../../test/tauri";
import { SettingsModelsScreen } from "../SettingsModelsScreen";
import { formatBytes } from "./SettingsPrivacyScreen";
import { SettingsLayout } from "./SettingsLayout";
import { SettingsPrivacyScreen } from "./SettingsPrivacyScreen";

const reveal = vi.fn(async (_path: string) => {});
const openDialog = vi.fn(async (_options: unknown): Promise<string | null> => "/tmp/out");

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: (p: string) => reveal(p) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (o: unknown) => openDialog(o), save: vi.fn() }));

const DATA = "/Users/x/Library/Application Support/app.morsel";

beforeEach(() => {
  resetTauri();
  reveal.mockClear();
  openDialog.mockClear();
  responses.data_info = { path: DATA, notes: 48, bytes: 6 * 1024 * 1024 };
  responses.export_all = 3;
  responses.get_settings = {
    notesBaseUrl: "https://openrouter.ai/api/v1", notesModel: "m", quickModel: "", transcribeMode: "local", localModel: "parakeet",
    transcribeBaseUrl: "https://api.groq.com/openai/v1", transcribeModel: "whisper-large-v3-turbo", language: "auto",
  };
  responses.has_api_key = true;
  responses.model_status = true;
  responses.usage_summary = { promptTokens: 1200, completionTokens: 300, cost: null };
});

function renderAt(path: string) {
  render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route element={<SettingsLayout />}>
          <Route path="/settings/models" element={<SettingsModelsScreen />} />
          <Route path="/settings/privacy" element={<SettingsPrivacyScreen />} />
        </Route>
        <Route path="/" element={<p>home page</p>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("Settings", () => {
  it("settings_nav_links_and_back", async () => {
    renderAt("/settings/privacy");
    const nav = screen.getByRole("navigation", { name: "Settings" });
    expect(within(nav).getAllByRole("link").map((l) => l.textContent)).toEqual([
      "Back to Morsel", "Models", "Transcription", "Calendar", "Privacy and data", "Shortcuts",
    ]);
    expect(within(nav).getByRole("link", { name: "Privacy and data" })).toHaveAttribute("aria-current", "page");
    fireEvent.click(within(nav).getByRole("link", { name: "Back to Morsel" }));
    expect(await screen.findByText("home page")).toBeInTheDocument();
  });

  it("privacy_shows_data_info", async () => {
    renderAt("/settings/privacy");
    expect(await screen.findByText(`${DATA}, 48 notes, 6 MB`)).toBeInTheDocument();
    expect(screen.getByText("Morsel never saves audio. It keeps only the text.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show in Finder" }));
    await waitFor(() => expect(reveal).toHaveBeenCalledWith(DATA));
  });

  it("export_picks_folder_and_reports_count", async () => {
    renderAt("/settings/privacy");
    fireEvent.click(await screen.findByRole("button", { name: "Export" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("export_all", { dir: "/tmp/out" }));
    expect(openDialog).toHaveBeenCalledWith(expect.objectContaining({ directory: true }));
    expect(await screen.findByText("Exported 3 notes.")).toBeInTheDocument();
  });

  it("export_cancelled_does_nothing", async () => {
    openDialog.mockResolvedValueOnce(null);
    renderAt("/settings/privacy");
    fireEvent.click(await screen.findByRole("button", { name: "Export" }));
    await waitFor(() => expect(openDialog).toHaveBeenCalled());
    expect(invoke.mock.calls.some(([c]) => c === "export_all")).toBe(false);
  });

  it("delete_all_asks_then_deletes", async () => {
    renderAt("/settings/privacy");
    fireEvent.click(await screen.findByRole("button", { name: "Delete all data" }));
    const dialog = screen.getByRole("dialog", { name: "Delete all data?" });
    expect(within(dialog).getByText(/You cannot undo this./)).toBeInTheDocument();
    expect(invoke.mock.calls.some(([c]) => c === "delete_all_data")).toBe(false);
    responses.data_info = { path: DATA, notes: 0, bytes: 40_960 };
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete all data" }));
    await waitFor(() => expect(invoke.mock.calls.some(([c]) => c === "delete_all_data")).toBe(true));
    expect(await screen.findByText("All data is deleted.")).toBeInTheDocument();
    expect(await screen.findByText(`${DATA}, 0 notes, 40 KB`)).toBeInTheDocument();
  });

  it("models_shows_usage_and_hides_unknown_cost", async () => {
    renderAt("/settings/models");
    expect(await screen.findByText("Usage, last 30 days: 1,500 tokens")).toBeInTheDocument();
    expect(screen.queryByText(/\$/)).not.toBeInTheDocument();
  });

  it("models_shows_cost_when_known", async () => {
    responses.usage_summary = { promptTokens: 10, completionTokens: 5, cost: 0.0421 };
    renderAt("/settings/models");
    expect(await screen.findByText("Usage, last 30 days: 15 tokens, $0.04")).toBeInTheDocument();
  });

  it("formats_bytes", () => {
    expect(formatBytes(512)).toBe("1 KB");
    expect(formatBytes(40_960)).toBe("40 KB");
    expect(formatBytes(6 * 1024 * 1024)).toBe("6 MB");
    expect(formatBytes(1.5 * 1024 * 1024 * 1024)).toBe("1.5 GB");
  });
});
