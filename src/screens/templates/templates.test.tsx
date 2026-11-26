import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, resetTauri, responses } from "../../test/tauri";
import { TemplatesScreen } from "./TemplatesScreen";

vi.mock("@tauri-apps/api/core", async () => ({ invoke: (await import("../../test/tauri")).invoke }));
vi.mock("@tauri-apps/api/event", async () => ({ listen: (await import("../../test/tauri")).listen }));

const general = {
  id: 1, name: "General", description: "Any meeting", builtIn: true, uses: 0, instructions: "Be clear.",
  sections: [{ name: "Summary", hint: "The meeting in short" }],
};
const product = {
  id: 5, name: "Product sync", description: "Weekly product meetings", builtIn: true, uses: 14, instructions: "Name owners.",
  sections: [{ name: "Decisions", hint: "What the team agreed" }, { name: "Risks", hint: "What can delay the plan" }],
};
const mine = {
  id: 7, name: "Design crit", description: "", builtIn: false, uses: 1, instructions: "",
  sections: [{ name: "Feedback", hint: "" }],
};

function calls(cmd: string) {
  return invoke.mock.calls.filter(([c]) => c === cmd).map(([, args]) => args as Record<string, unknown>);
}

beforeEach(() => {
  resetTauri();
  responses.list_templates = [general, product, mine];
  responses.save_template = 5;
});

function renderTemplates() {
  render(<MemoryRouter><TemplatesScreen /></MemoryRouter>);
}

describe("TemplatesScreen", () => {
  it("shows_templates_with_use_counts", async () => {
    renderTemplates();
    expect(await screen.findByText("Used 14 times")).toBeInTheDocument();
    expect(screen.getByText("Default for new notes")).toBeInTheDocument();
    expect(screen.getByText("Used 1 time")).toBeInTheDocument();
    expect(screen.getByLabelText("Name")).toHaveValue("General");
    expect(screen.getByLabelText("When to use")).toHaveValue("Any meeting");
  });

  it("save_sends_sections_and_hints", async () => {
    renderTemplates();
    fireEvent.click(await screen.findByRole("button", { name: /Product sync/ }));
    fireEvent.change(screen.getByLabelText("Section 2 hint"), { target: { value: "Things that can slip" } });
    fireEvent.click(screen.getByRole("button", { name: "Add section" }));
    fireEvent.change(screen.getByLabelText("Section 3 name"), { target: { value: "Action items" } });
    fireEvent.click(screen.getByRole("button", { name: "Move Risks up" }));
    fireEvent.click(screen.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(calls("save_template")).toHaveLength(1));
    expect(calls("save_template")[0]).toEqual({
      id: 5,
      input: {
        name: "Product sync", description: "Weekly product meetings", instructions: "Name owners.",
        sections: [
          { name: "Risks", hint: "Things that can slip" },
          { name: "Decisions", hint: "What the team agreed" },
          { name: "Action items", hint: "" },
        ],
      },
    });
    expect(await screen.findByText("Saved.")).toBeInTheDocument();
  });

  it("hint_without_name_waits_for_a_name", async () => {
    renderTemplates();
    fireEvent.click(await screen.findByRole("button", { name: /Product sync/ }));
    fireEvent.click(screen.getByRole("button", { name: "Add section" }));
    fireEvent.change(screen.getByLabelText("Section 3 hint"), { target: { value: "what the customer asked for" } });
    fireEvent.click(screen.getByRole("button", { name: "Save template" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Give section 3 a name.");
    const field = screen.getByLabelText("Section 3 name");
    expect(field).toHaveAttribute("aria-invalid", "true");
    expect(field).toHaveAccessibleDescription("Give section 3 a name.");
    expect(field).toHaveFocus();
    expect(screen.getByLabelText("Section 1 name")).not.toHaveAttribute("aria-invalid", "true");
    expect(calls("save_template")).toHaveLength(0);
    expect(screen.queryByText("Saved.")).not.toBeInTheDocument();
    fireEvent.change(field, { target: { value: "Asks" } });
    expect(field).not.toHaveAttribute("aria-invalid", "true");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(calls("save_template")).toHaveLength(1));
    expect(await screen.findByText("Saved.")).toBeInTheDocument();
  });

  it("hint_without_name_names_every_row", async () => {
    renderTemplates();
    fireEvent.click(await screen.findByRole("button", { name: /Product sync/ }));
    fireEvent.click(screen.getByRole("button", { name: "Add section" }));
    fireEvent.click(screen.getByRole("button", { name: "Add section" }));
    fireEvent.change(screen.getByLabelText("Section 1 name"), { target: { value: "" } });
    fireEvent.change(screen.getByLabelText("Section 4 hint"), { target: { value: "Who does what" } });
    fireEvent.click(screen.getByRole("button", { name: "Save template" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/^Give sections 1 and 4 a name\.$/);
    expect(screen.getByLabelText("Section 4 name")).toHaveAttribute("aria-invalid", "true");
  });

  it("empty_section_row_saves_without_error", async () => {
    renderTemplates();
    fireEvent.click(await screen.findByRole("button", { name: /Product sync/ }));
    fireEvent.click(screen.getByRole("button", { name: "Add section" }));
    fireEvent.click(screen.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(calls("save_template")).toHaveLength(1));
    expect(await screen.findByText("Saved.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("no_named_section_says_add_one", async () => {
    renderTemplates();
    await screen.findByText("Used 14 times");
    fireEvent.click(screen.getByRole("button", { name: "New template" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Retro" } });
    fireEvent.click(screen.getByRole("button", { name: "Save template" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/^Add at least one section\.$/);
    expect(calls("save_template")).toHaveLength(0);
  });

  it("no_template_name_says_name_it_first", async () => {
    renderTemplates();
    await screen.findByText("Used 14 times");
    fireEvent.click(screen.getByRole("button", { name: "New template" }));
    fireEvent.click(screen.getByRole("button", { name: "Save template" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/^Give the template a name\.$/);
    expect(calls("save_template")).toHaveLength(0);
  });

  it("remove_section_drops_it", async () => {
    renderTemplates();
    fireEvent.click(await screen.findByRole("button", { name: /Product sync/ }));
    fireEvent.click(screen.getByRole("button", { name: "Remove Decisions" }));
    expect(screen.queryByDisplayValue("Decisions")).not.toBeInTheDocument();
    expect(screen.getByDisplayValue("Risks")).toBeInTheDocument();
  });

  it("new_template_starts_empty", async () => {
    responses.save_template = 8;
    renderTemplates();
    await screen.findByText("Used 14 times");
    fireEvent.click(screen.getByRole("button", { name: "New template" }));
    expect(screen.getByLabelText("Name")).toHaveValue("");
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Retro" } });
    fireEvent.change(screen.getByLabelText("Section 1 name"), { target: { value: "Went well" } });
    fireEvent.click(screen.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(calls("save_template")).toHaveLength(1));
    expect(calls("save_template")[0]).toMatchObject({ id: null, input: { name: "Retro", sections: [{ name: "Went well", hint: "" }] } });
  });

  it("reset_only_for_built_in", async () => {
    renderTemplates();
    await screen.findByText("Used 14 times");
    expect(screen.getByRole("button", { name: "Reset" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Delete" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(calls("reset_template")).toEqual([{ id: 1 }]));
    fireEvent.click(screen.getByRole("button", { name: /Design crit/ }));
    expect(screen.queryByRole("button", { name: "Reset" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument();
  });

  it("delete_asks_first", async () => {
    renderTemplates();
    fireEvent.click(await screen.findByRole("button", { name: /Design crit/ }));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = screen.getByRole("dialog", { name: "Delete this template?" });
    expect(calls("delete_template")).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete template" }));
    await waitFor(() => expect(calls("delete_template")).toEqual([{ id: 7 }]));
  });

  it("duplicate_selects_the_copy", async () => {
    responses.duplicate_template = 9;
    renderTemplates();
    await screen.findByText("Used 14 times");
    responses.list_templates = [general, product, mine, { ...general, id: 9, name: "General copy", builtIn: false }];
    fireEvent.click(screen.getByRole("button", { name: "Duplicate" }));
    await waitFor(() => expect(screen.getByLabelText("Name")).toHaveValue("General copy"));
  });
});
