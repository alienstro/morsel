import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import App from "./App";

describe("App", () => {
  it("renders the Morsel window title", () => {
    render(<App />);
    expect(document.title).toBe("Morsel");
    expect(screen.getByText("Morsel")).toBeInTheDocument();
  });
});
