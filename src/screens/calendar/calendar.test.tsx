import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emit, invoke, resetTauri, responses } from "../../test/tauri";
import { CalendarStep } from "../onboarding/CalendarStep";
import { SettingsCalendarScreen } from "../settings/SettingsCalendarScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));

const none = { state: "none", today: 0 };
const settings = {
  notesBaseUrl: "", notesModel: "", quickModel: "", transcribeMode: "local", transcribeBaseUrl: "",
  transcribeModel: "", language: "auto", onboarded: false, remindBeforeMeeting: true, globalShortcut: "CommandOrControl+Shift+M",
};

function calls(cmd: string) {
  return invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
}

beforeEach(() => {
  resetTauri();
  responses.calendar_status = { hasKey: false, google: none, outlook: none };
  responses.get_settings = settings;
});

function renderStep() {
  render(
    <MemoryRouter initialEntries={["/welcome/3"]}>
      <Routes>
        <Route path="/welcome/3" element={<CalendarStep />} />
        <Route path="/" element={<p>home page</p>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("CalendarPanel", () => {
  it("panel_saves_the_composio_key", async () => {
    renderStep();
    expect(await screen.findByRole("button", { name: "Connect Google Calendar" })).toBeDisabled();
    responses.calendar_status = { hasKey: true, google: none, outlook: none };
    fireEvent.change(screen.getByLabelText("Composio key"), { target: { value: "ck-123" } });
    fireEvent.click(screen.getByRole("button", { name: "Save key" }));
    await waitFor(() => expect(calls("set_api_key")).toEqual([{ account: "composio_api_key", key: "ck-123" }]));
    await waitFor(() => expect(screen.getByRole("button", { name: "Connect Google Calendar" })).toBeEnabled());
    expect(screen.getByLabelText("Composio key")).toHaveValue("");
  });

  it("user_api_key_is_refused_with_help", async () => {
    renderStep();
    await screen.findByRole("button", { name: "Connect Google Calendar" });
    fireEvent.change(screen.getByLabelText("Composio key"), { target: { value: "uak_123" } });
    fireEvent.click(screen.getByRole("button", { name: "Save key" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("This is a Composio user key. Morsel needs a project API key");
    expect(calls("set_api_key")).toEqual([]);
  });

  it("connect_calls_the_backend", async () => {
    responses.calendar_status = { hasKey: true, google: none, outlook: none };
    renderStep();
    fireEvent.click(await screen.findByRole("button", { name: "Connect Outlook Calendar" }));
    await waitFor(() => expect(calls("connect_calendar")).toEqual([{ provider: "outlook" }]));
  });

  it("connected_row_shows_the_meeting_count", async () => {
    responses.calendar_status = { hasKey: true, google: { state: "connected", today: 3 }, outlook: { state: "pending", today: 0 } };
    renderStep();
    expect(await screen.findByText("Connected. 3 meetings today.")).toBeInTheDocument();
    expect(screen.getByText("Waiting for the sign-in")).toBeInTheDocument();
    responses.calendar_status = { hasKey: true, google: { state: "connected", today: 1 }, outlook: none };
    act(() => emit("calendar://changed", null));
    expect(await screen.findByText("Connected. 1 meeting today.")).toBeInTheDocument();
  });

  it("disconnect_calls_the_backend", async () => {
    responses.calendar_status = { hasKey: true, google: { state: "connected", today: 0 }, outlook: none };
    renderStep();
    fireEvent.click(await screen.findByRole("button", { name: "Disconnect Google Calendar" }));
    await waitFor(() => expect(calls("disconnect_calendar")).toEqual([{ provider: "google" }]));
  });
});

describe("CalendarStep", () => {
  it("finish_marks_onboarding_done", async () => {
    renderStep();
    fireEvent.click(await screen.findByRole("button", { name: "Finish" }));
    expect(await screen.findByText("home page")).toBeInTheDocument();
    expect(calls("save_settings")[0].settings).toMatchObject({ onboarded: true });
  });

  it("skip_marks_onboarding_done", async () => {
    renderStep();
    fireEvent.click(await screen.findByRole("button", { name: "Skip for now" }));
    expect(await screen.findByText("home page")).toBeInTheDocument();
    expect(calls("save_settings")[0].settings).toMatchObject({ onboarded: true });
  });
});

describe("SettingsCalendarScreen", () => {
  it("reminder_switch_saves_the_setting", async () => {
    render(<MemoryRouter><SettingsCalendarScreen /></MemoryRouter>);
    const box = await screen.findByRole("switch", { name: "Remind me 1 minute before a meeting" });
    expect(box).toBeChecked();
    fireEvent.click(box);
    await waitFor(() => expect(calls("save_settings")[0].settings).toMatchObject({ remindBeforeMeeting: false }));
    expect(box).not.toBeChecked();
  });
});
