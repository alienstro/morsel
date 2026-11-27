import { useEffect, useId, useRef } from "react";
import "../screens/library.css";

type Props = {
  title: string;
  body: string;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
};

/**
 * An in-app confirmation for a destructive action. Focus starts on "Cancel", Tab stays in the dialog,
 * Escape cancels from any place, and focus goes back to the opener on close (issue #26).
 */
export function ConfirmDialog({ title, body, confirmLabel, onConfirm, onCancel }: Props) {
  const titleId = useId();
  const cancelRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    cancelRef.current?.focus();
    return () => {
      // Issue #26: do not take focus back from an element that a close handler focused.
      if (document.activeElement === document.body || !document.activeElement) opener?.focus();
    };
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.key === "Escape") onCancel();
      if (e.key !== "Tab") return;
      // Issue #26: the dialog has two buttons, so Tab and Shift+Tab both go to the other one.
      e.preventDefault();
      (document.activeElement === cancelRef.current ? confirmRef : cancelRef).current?.focus();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onCancel]);
  return (
    <div className="dialog-backdrop" onClick={onCancel}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id={titleId}>{title}</h2>
        <p>{body}</p>
        <div className="dialog-actions">
          <button ref={cancelRef} type="button" className="btn" onClick={onCancel}>Cancel</button>
          <button ref={confirmRef} type="button" className="btn btn-danger-fill" onClick={onConfirm}>{confirmLabel}</button>
        </div>
      </div>
    </div>
  );
}
