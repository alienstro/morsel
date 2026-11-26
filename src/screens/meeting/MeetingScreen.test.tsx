import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emit, invoke, resetTauri, responses } from "../../test/tauri";
import { MeetingScreen } from "./MeetingScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));
vi.mock("./Notepad", () => ({
  Notepad: ({ initial, onChange }: { initial: string; onChange: (text: string) => void }) => (
    <textarea aria-label="Rough notes" defaultValue={initial} onChange={(e) => onChange(e.target.value)} />
  ),
}));

const detail = {
  summary: { id: 1, title: "Weekly product sync", templateId: 5, templateName: "Product sync", status: "recording", startedAt: Date.now() },
  lines: [],
  userNotes: "",
  note: null,
};

async function renderScreen() {
  render(
    <MemoryRouter initialEntries={["/meeting/1"]}>
      <Routes>
        <Route path="/meeting/:id" element={<MeetingScreen />} />
        <Route path="/meeting/:id/writing" element={<p>writing page</p>} />
      </Routes>
    </MemoryRouter>,
  );
  await screen.findByRole("heading", { name: "Weekly product sync" });
}

beforeEach(() => {
  resetTauri();
  sessionStorage.clear();
  responses.get_meeting = detail;
  responses.list_meetings = [detail.summary];
});

afterEach(() => {
  vi.useRealTimers();
});

/** Renders at a fixed start time. Fake timers still move with real time, so `findBy` works. */
async function renderAt(status: string, startedAt = 1_000_000) {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  vi.setSystemTime(startedAt);
  responses.get_meeting = { ...detail, summary: { ...detail.summary, status, startedAt } };
  await renderScreen();
  vi.setSystemTime(startedAt);
}

function wait(ms: number) {
  act(() => vi.advanceTimersByTime(ms));
}

function setStatus(status: string) {
  act(() => emit("meeting://status", { meetingId: 1, status }));
}

describe("MeetingScreen", () => {
  it("clock_does_not_count_the_paused_time", async () => {
    await renderAt("recording");
    wait(3000);
    setStatus("paused");
    wait(10_000);
    expect(screen.getByText("0:03")).toBeInTheDocument();
    setStatus("recording");
    wait(1000);
    expect(screen.getByText("0:04")).toBeInTheDocument();
  });

  it("clock_adds_only_the_recorded_parts_after_two_pauses", async () => {
    await renderAt("recording");
    wait(3000);
    setStatus("paused");
    wait(10_000);
    setStatus("recording");
    wait(2000);
    setStatus("paused");
    wait(20_000);
    setStatus("recording");
    wait(1000);
    expect(screen.getByText("0:06")).toBeInTheDocument();
  });

  it("clock_after_a_reload_while_paused_shows_the_recorded_time", async () => {
    await renderAt("recording");
    wait(3000);
    setStatus("paused");
    wait(10_000);
    cleanup();
    responses.get_meeting = { ...detail, summary: { ...detail.summary, status: "paused", startedAt: 1_000_000 } };
    await renderScreen();
    wait(5000);
    expect(screen.getByText("0:03")).toBeInTheDocument();
  });

  it("clock_shows_hours_after_60_minutes", async () => {
    await renderAt("recording");
    // The 1 s tick started at mount, a few ms before renderAt reset the time, so wait one more tick.
    wait(3_726_000);
    expect(screen.getByText("1:02:05")).toBeInTheDocument();
  });

  it("new_transcript_event_adds_a_line_with_speaker_label", async () => {
    await renderScreen();
    act(() => emit("meeting://transcript", { meetingId: 1, speaker: "others", startMs: 62_000, endMs: 70_000, text: "Probably one week late." }));
    expect(await screen.findByText("Probably one week late.")).toBeInTheDocument();
    expect(screen.getByText("Others, 1:02")).toBeInTheDocument();
  });

  it("hide_and_cmd_t_toggle_the_panel", async () => {
    await renderScreen();
    expect(screen.getByRole("complementary", { name: "Transcript" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Hide transcript" }));
    expect(screen.queryByRole("complementary", { name: "Transcript" })).not.toBeInTheDocument();
    fireEvent.keyDown(window, { key: "t", metaKey: true });
    expect(screen.getByRole("complementary", { name: "Transcript" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Hide transcript" }));
    fireEvent.click(screen.getByRole("button", { name: /Show transcript/ }));
    expect(screen.getByRole("complementary", { name: "Transcript" })).toBeInTheDocument();
  });

  it("rough_notes_save_2s_after_typing", async () => {
    await renderScreen();
    vi.useFakeTimers();
    fireEvent.change(screen.getByLabelText("Rough notes"), { target: { value: "budget ok" } });
    act(() => vi.advanceTimersByTime(1900));
    expect(invoke).not.toHaveBeenCalledWith("save_user_notes", expect.anything());
    act(() => vi.advanceTimersByTime(200));
    expect(invoke).toHaveBeenCalledWith("save_user_notes", { meetingId: 1, body: "budget ok" });
  });

  it("stop_saves_notes_then_writes_then_navigates", async () => {
    await renderScreen();
    fireEvent.change(screen.getByLabelText("Rough notes"), { target: { value: "ask Priya" } });
    fireEvent.click(screen.getByRole("button", { name: "Stop and write notes" }));
    expect(await screen.findByText("writing page")).toBeInTheDocument();
    const calls = invoke.mock.calls.map((c) => c[0]).filter((c) => ["save_user_notes", "stop_meeting", "write_notes"].includes(c));
    expect(calls).toEqual(["save_user_notes", "stop_meeting", "write_notes"]);
    expect(invoke).toHaveBeenCalledWith("write_notes", { meetingId: 1, templateId: 5 });
  });

  it("after_stop_opens_the_writing_page_without_write_notes", async () => {
    responses.stop_meeting = "after";
    await renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Stop and write notes" }));
    expect(await screen.findByText("writing page")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("stop_meeting");
    expect(invoke).not.toHaveBeenCalledWith("write_notes", expect.anything());
  });

  it("system_audio_warning_shows_in_the_status_bar", async () => {
    await renderScreen();
    act(() => emit("meeting://status", { meetingId: 1, status: "recording", warning: "Others are not recorded: no permission" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Others are not recorded");
  });

  it("meeting_panel_asks_about_this_meeting", async () => {
    responses.ask = { id: null, question: "Who is blocked?", answer: "Leo is blocked on the API.", sources: [] };
    await renderScreen();
    fireEvent.change(screen.getByLabelText("Ask about this meeting"), { target: { value: "Who is blocked?" } });
    fireEvent.click(screen.getByRole("button", { name: "Ask" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ask", { question: "Who is blocked?", range: "all", meetingId: 1 }));
    expect(await screen.findByText("Leo is blocked on the API.")).toBeInTheDocument();
  });
});
