import { useEffect, useState } from "react";
import { api, errorMessage } from "../../lib/api";

const SYMBOLS: Record<string, string> = {
  CommandOrControl: "⌘", CmdOrCtrl: "⌘", Command: "⌘", Cmd: "⌘", Super: "⌘",
  Control: "⌃", Ctrl: "⌃", Alt: "⌥", Option: "⌥", Shift: "⇧",
};
const MODIFIER_KEYS = ["Meta", "Control", "Alt", "Shift"];

/** "CommandOrControl+Shift+M" → "⌘⇧M". */
export function formatShortcut(accelerator: string): string {
  return accelerator.split("+").map((part) => SYMBOLS[part] ?? part).join("");
}

/** The accelerator of a key press, or null for a modifier key alone. */
function accelerator(e: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.includes(e.key)) return null;
  const key = e.code.startsWith("Key") ? e.code.slice(3) : e.code.startsWith("Digit") ? e.code.slice(5) : e.code;
  const parts = [e.metaKey && "CommandOrControl", e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", key];
  return parts.filter(Boolean).join("+");
}

const IN_APP: [string, string][] = [
  ["⌘K", "Ask about any meeting"],
  ["⌘T", "Hide or show the transcript in a meeting"],
];

/** Settings: the global shortcut that starts a note, and the shortcuts inside Morsel. */
export function SettingsShortcutsScreen() {
  const [current, setCurrent] = useState<string | null>(null);
  const [capturing, setCapturing] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    api.getSettings().then((s) => setCurrent(s.globalShortcut)).catch((e) => setMessage({ ok: false, text: errorMessage(e) }));
  }, []);

  const save = async (next: string) => {
    setMessage(null);
    try {
      await api.setGlobalShortcut(next);
      setCurrent(next);
      setMessage({ ok: true, text: "Shortcut saved." });
    } catch (e) {
      setMessage({ ok: false, text: errorMessage(e) });
    }
  };

  useEffect(() => {
    if (!capturing) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      if (e.key === "Escape") {
        setCapturing(false);
        return;
      }
      const next = accelerator(e);
      if (!next) return;
      setCapturing(false);
      void save(next);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [capturing]);

  return (
    <div className="page">
      <h1 className="serif page-title">Shortcuts</h1>
      <section className="settings-section" aria-labelledby="global-shortcut">
        <h2 id="global-shortcut" className="section-title">Start a note from any app</h2>
        <p className="muted small" style={{ margin: 0 }}>
          This shortcut works also when Morsel is in the background. It opens the meeting that records now, or starts a new note.
        </p>
        <div className="row">
          {current && <kbd className="shortcut-key">{formatShortcut(current)}</kbd>}
          <button type="button" className="btn" onClick={() => { setMessage(null); setCapturing(true); }} disabled={capturing}>Change</button>
          <button type="button" className="btn" onClick={() => save("CommandOrControl+Shift+M")}>Reset</button>
        </div>
        {capturing && <p role="status" className="small" style={{ margin: 0 }}>Press the new shortcut. Esc cancels.</p>}
        {message && <p role={message.ok ? "status" : "alert"} className={message.ok ? "small" : "error-text small"} style={{ margin: 0 }}>{message.text}</p>}
      </section>
      <section className="settings-section" aria-labelledby="in-app-shortcuts">
        <h2 id="in-app-shortcuts" className="section-title">In Morsel</h2>
        <ul className="shortcut-list">
          {IN_APP.map(([keys, what]) => (
            <li key={keys}><kbd className="shortcut-key">{keys}</kbd><span>{what}</span></li>
          ))}
        </ul>
      </section>
    </div>
  );
}
