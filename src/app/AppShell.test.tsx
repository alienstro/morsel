import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { AppShell } from "./AppShell";

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <AppShell recent={[{ id: 7, title: "1:1 with Sam" }]}>
        <p>content</p>
      </AppShell>
    </MemoryRouter>,
  );
}

describe("AppShell", () => {
  it("shows all sidebar links", () => {
    renderAt("/");
    const nav = screen.getByRole("navigation", { name: "Main" });
    for (const name of ["Home", "Notes", "Templates", "Ask Morsel", "Settings", "1:1 with Sam"]) {
      expect(nav).toContainElement(screen.getByRole("link", { name }));
    }
    expect(screen.getByText("Recent")).toBeInTheDocument();
    expect(screen.getByText("content")).toBeInTheDocument();
  });

  it("marks the current page with aria-current", () => {
    renderAt("/templates");
    expect(screen.getByRole("link", { name: "Templates" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Home" })).not.toHaveAttribute("aria-current");
  });

  it("New note button has an aria-label", () => {
    renderAt("/");
    expect(screen.getByRole("button", { name: "New note" })).toBeInTheDocument();
  });
});
