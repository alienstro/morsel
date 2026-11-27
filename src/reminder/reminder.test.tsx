import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, resetTauri, responses } from "../test/tauri";
import { ReminderApp } from "./ReminderApp";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../test/tauri")).invoke }));

beforeEach(() => {
  resetTauri();
  responses.reminder_event = {
    id: "e1", provider: "google", title: "Weekly product sync", start: Date.now() + 50_000, end: Date.now() + 1_800_000,
    link: "https://meet.google.com/abc", people: ["Sam", "Priya", "Leo"],
  };
});

describe("ReminderApp", () => {
  it("reminder_shows_title_and_people", async () => {
    render(<ReminderApp />);
    expect(await screen.findByText("Weekly product sync in 1 min")).toBeInTheDocument();
    expect(screen.getByText("Google Meet with Sam, Priya and Leo")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("start_notes_calls_the_backend", async () => {
    responses.start_from_reminder = 7;
    render(<ReminderApp />);
    fireEvent.click(await screen.findByRole("button", { name: "Start notes" }));
    await waitFor(() => expect(invoke.mock.calls.some(([c]) => c === "start_from_reminder")).toBe(true));
  });

  it("start_error_shows_in_the_reminder", async () => {
    responses.start_from_reminder = () => Promise.reject({ code: "already_recording", message: "a meeting is already recording" });
    render(<ReminderApp />);
    fireEvent.click(await screen.findByRole("button", { name: "Start notes" }));
    expect(await screen.findByText("A meeting is already recording. Stop it first.")).toBeInTheDocument();
  });

  it("dismiss_closes_the_reminder", async () => {
    render(<ReminderApp />);
    fireEvent.click(await screen.findByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(invoke.mock.calls.some(([c]) => c === "dismiss_reminder")).toBe(true));
  });

  it("reminder_css_does_not_style_the_main_window", () => {
    // main.tsx always imports ReminderApp, so its page rules also reach the main window (#16).
    expect(getComputedStyle(document.body).overflow).not.toBe("hidden");
    document.documentElement.dataset.window = "reminder";
    try {
      expect(getComputedStyle(document.body).overflow).toBe("hidden");
    } finally {
      delete document.documentElement.dataset.window;
    }
  });
});
