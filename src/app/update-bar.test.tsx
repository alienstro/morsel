import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UpdateBar } from "./UpdateBar";

const check = vi.fn();
const relaunch = vi.fn();
vi.mock("@tauri-apps/plugin-updater", () => ({ check: () => check() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: () => relaunch() }));

beforeEach(() => {
  check.mockReset();
  relaunch.mockReset();
});

describe("UpdateBar", () => {
  it("update_bar_shows_a_new_version", async () => {
    check.mockResolvedValue({ version: "0.2.0", downloadAndInstall: vi.fn() });
    render(<UpdateBar />);
    expect(await screen.findByText("Morsel 0.2.0 is ready.")).toBeInTheDocument();
  });

  it("update_bar_installs_only_after_a_click", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    check.mockResolvedValue({ version: "0.2.0", downloadAndInstall });
    render(<UpdateBar />);
    const button = await screen.findByRole("button", { name: "Update and restart" });
    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
    fireEvent.click(button);
    await waitFor(() => expect(relaunch).toHaveBeenCalledTimes(1));
    expect(downloadAndInstall).toHaveBeenCalledTimes(1);
  });

  it("update_check_failure_shows_nothing", async () => {
    check.mockRejectedValue(new Error("offline"));
    const { container } = render(<UpdateBar />);
    await waitFor(() => expect(check).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("no_update_shows_nothing", async () => {
    check.mockResolvedValue(null);
    const { container } = render(<UpdateBar />);
    await waitFor(() => expect(check).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("failed_install_shows_a_message", async () => {
    check.mockResolvedValue({ version: "0.2.0", downloadAndInstall: vi.fn().mockRejectedValue(new Error("bad signature")) });
    render(<UpdateBar />);
    fireEvent.click(await screen.findByRole("button", { name: "Update and restart" }));
    expect(await screen.findByText("The update failed. Try again later.")).toBeInTheDocument();
    expect(relaunch).not.toHaveBeenCalled();
  });
});
