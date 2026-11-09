import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { resetTauri, responses } from "./test/tauri";
import App from "./App";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("./test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("./test/tauri")).listen }));

beforeEach(() => {
  resetTauri();
  responses.get_settings = { onboarded: true };
  responses.list_templates = [];
  responses.list_meetings = [];
  responses.search_notes = { items: [], total: 0 };
});

describe("App", () => {
  it("renders the Morsel window title", async () => {
    render(<App />);
    expect(document.title).toBe("Morsel");
    expect(await screen.findByText("Morsel")).toBeInTheDocument();
  });
});
