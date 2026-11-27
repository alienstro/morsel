import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { ConfirmDialog } from "./ConfirmDialog";

function Page({ onCancel = () => {} }: { onCancel?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button">New note</button>
      <button type="button" onClick={() => setOpen(true)}>Delete</button>
      {open && (
        <ConfirmDialog
          title="Delete this note?"
          body="You cannot undo this."
          confirmLabel="Delete note"
          onConfirm={() => setOpen(false)}
          onCancel={() => { onCancel(); setOpen(false); }}
        />
      )}
    </>
  );
}

async function openDialog(onCancel?: () => void) {
  const user = userEvent.setup();
  render(<Page onCancel={onCancel} />);
  await user.click(screen.getByRole("button", { name: "Delete" }));
  return user;
}

describe("ConfirmDialog", () => {
  it("confirm_dialog_keeps_tab_inside_and_escape_closes", async () => {
    const onCancel = vi.fn();
    const user = await openDialog(onCancel);
    const cancel = screen.getByRole("button", { name: "Cancel" });
    const confirm = screen.getByRole("button", { name: "Delete note" });
    expect(cancel).toHaveFocus();
    await user.tab();
    expect(confirm).toHaveFocus();
    await user.tab();
    expect(cancel).toHaveFocus();
    await user.tab();
    expect(confirm).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("confirm_dialog_shift_tab_from_cancel_goes_to_confirm", async () => {
    const user = await openDialog();
    await user.tab({ shift: true });
    expect(screen.getByRole("button", { name: "Delete note" })).toHaveFocus();
  });

  it("confirm_dialog_escape_closes_when_focus_is_outside", async () => {
    const user = await openDialog();
    screen.getByRole("button", { name: "New note" }).focus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("confirm_dialog_returns_focus_to_the_opener", async () => {
    const user = await openDialog();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: "Delete" })).toHaveFocus();
  });

  it("confirm_dialog_escape_returns_focus_to_the_opener", async () => {
    const user = await openDialog();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("button", { name: "Delete" })).toHaveFocus();
  });

  it("confirm_dialog_closes_on_backdrop_click", async () => {
    await openDialog();
    fireEvent.click(screen.getByRole("dialog").parentElement!);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
