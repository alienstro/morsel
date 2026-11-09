import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, resetTauri, responses } from "../../test/tauri";
import { AskScreen } from "./AskScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));

const answer = {
  id: 5,
  question: "What did Acme say about the price?",
  answer: "Acme thinks the price is fair for the first year.\n\n- They want a discount above 50 seats.",
  sources: [
    { meetingId: 3, title: "Acme discovery call", startedAt: new Date(2026, 10, 2, 11, 0).getTime() },
    { meetingId: 6, title: "Acme intro call", startedAt: new Date(2026, 9, 29, 14, 0).getTime() },
  ],
};

function askCalls() {
  return invoke.mock.calls.filter(([c]) => c === "ask").map(([, a]) => a);
}

beforeEach(() => {
  resetTauri();
  responses.list_asks = [{ ...answer, id: 2, question: "What did we decide on pricing?", answer: "Ten percent off.", sources: [] }];
  responses.ask = answer;
  responses.get_ask = { ...answer, id: 2, question: "What did we decide on pricing?", answer: "Ten percent off.", sources: [] };
});

function renderAt(path: string) {
  render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/ask" element={<AskScreen />} />
        <Route path="/ask/:id" element={<AskScreen />} />
        <Route path="/note/:id" element={<p>note page</p>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("AskScreen", () => {
  it("ask_sends_question_and_range", async () => {
    renderAt("/ask");
    const ask = screen.getByRole("button", { name: "Ask" });
    expect(ask).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Ask about your meetings"), { target: { value: "What did Acme say about the price?" } });
    fireEvent.change(screen.getByLabelText("Time range"), { target: { value: "week" } });
    fireEvent.click(ask);
    await waitFor(() => expect(askCalls()).toEqual([{ question: "What did Acme say about the price?", range: "week", meetingId: null }]));
  });

  it("answer_shows_source_links", async () => {
    renderAt("/ask");
    fireEvent.change(screen.getByLabelText("Ask about your meetings"), { target: { value: "price?" } });
    fireEvent.click(screen.getByRole("button", { name: "Ask" }));
    expect(await screen.findByText("Acme thinks the price is fair for the first year.")).toBeInTheDocument();
    expect(screen.getByText("They want a discount above 50 seats.")).toBeInTheDocument();
    expect(screen.getByText("From 2 meetings")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("link", { name: "Acme discovery call, 2 Nov" }));
    expect(await screen.findByText("note page")).toBeInTheDocument();
  });

  it("query_param_asks_at_once", async () => {
    renderAt("/ask?q=pricing");
    await waitFor(() => expect(askCalls()).toEqual([{ question: "pricing", range: "all", meetingId: null }]));
    expect(await screen.findByText("Acme thinks the price is fair for the first year.")).toBeInTheDocument();
  });

  it("earlier_question_opens_saved_answer", async () => {
    renderAt("/ask");
    fireEvent.click(await screen.findByRole("link", { name: "What did we decide on pricing?" }));
    expect(await screen.findByText("Ten percent off.")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("get_ask", { id: 2 });
    expect(askCalls()).toHaveLength(0);
  });

  it("error_shows_message", async () => {
    responses.ask = () => Promise.reject({ code: "invalid", message: "add the notes API key in Settings first" });
    renderAt("/ask");
    fireEvent.change(screen.getByLabelText("Ask about your meetings"), { target: { value: "anything" } });
    fireEvent.click(screen.getByRole("button", { name: "Ask" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("add the notes API key in Settings first");
  });
});
