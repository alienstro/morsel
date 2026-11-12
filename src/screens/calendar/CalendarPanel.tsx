import { type FormEvent, useCallback, useEffect, useState } from "react";
import { api, errorMessage, onCalendarChanged, type CalendarStatus, type Provider, type ProviderStatus } from "../../lib/api";

const NAMES: Record<Provider, string> = { google: "Google Calendar", outlook: "Outlook Calendar" };

const PROJECT_KEY_HELP =
  "Morsel needs a project API key that starts with ak_: on composio.dev, open Settings, then Project Settings, and copy a key from API Keys.";

// Composio rejects these key kinds in the x-api-key header that Morsel sends, so the panel stops them before the save.
const WRONG_KEY_KINDS: ReadonlyArray<readonly [prefix: string, kind: string]> = [
  ["uak_", "user"],
  ["ck_", "Connect (MCP)"],
];

function stateText(s: ProviderStatus): string {
  if (s.state === "connected") return `Connected. ${s.today} ${s.today === 1 ? "meeting" : "meetings"} today.`;
  if (s.state === "pending") return "Waiting for the sign-in";
  return "Not connected";
}

/** The Composio key and one row for each calendar. Used by step 1.3 and by Settings. */
export function CalendarPanel() {
  const [status, setStatus] = useState<CalendarStatus | null>(null);
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api.calendarStatus().then(setStatus).catch((e) => setError(errorMessage(e)));
  }, []);

  useEffect(() => {
    load();
    const off = onCalendarChanged(load);
    window.addEventListener("focus", load);
    return () => {
      off.then((f) => f());
      window.removeEventListener("focus", load);
    };
  }, [load]);

  const saveKey = async (e: FormEvent) => {
    e.preventDefault();
    if (!key.trim()) return;
    const wrongKind = WRONG_KEY_KINDS.find(([prefix]) => key.trim().startsWith(prefix));
    if (wrongKind) {
      setError(`This is a Composio ${wrongKind[1]} key. ${PROJECT_KEY_HELP}`);
      return;
    }
    setError(null);
    try {
      await api.setApiKey("composio_api_key", key.trim());
      setKey("");
      load();
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  const run = async (action: () => Promise<void>) => {
    setError(null);
    try {
      await action();
      load();
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  if (!status) return error ? <p role="alert" className="error-text small">{error}</p> : null;

  return (
    <div className="calendar-panel">
      <form className="field" onSubmit={saveKey}>
        <label htmlFor="composio-key">Composio key</label>
        <div className="row">
          <input
            id="composio-key" type="password" className="text-input" autoComplete="off"
            placeholder={status.hasKey ? "Saved in the Keychain. Type a new key to change it." : ""}
            value={key} onChange={(e) => setKey(e.target.value)}
          />
          <button type="submit" className="btn" disabled={!key.trim()}>Save key</button>
        </div>
      </form>
      <div className="perm-list">
        {(["google", "outlook"] as Provider[]).map((p) => {
          const s = status[p];
          return (
            <div key={p} className="perm-row">
              <div className="perm-text">
                <div className="perm-name">{NAMES[p]}</div>
                <div className="muted small">{stateText(s)}</div>
              </div>
              {s.state === "connected" ? (
                <button type="button" className="btn" aria-label={`Disconnect ${NAMES[p]}`} onClick={() => run(() => api.disconnectCalendar(p))}>Disconnect</button>
              ) : (
                <button type="button" className="btn" aria-label={`Connect ${NAMES[p]}`} disabled={!status.hasKey} onClick={() => run(() => api.connectCalendar(p))}>
                  {s.state === "pending" ? "Try again" : "Connect"}
                </button>
              )}
            </div>
          );
        })}
      </div>
      {error && <p role="alert" className="error-text small" style={{ margin: 0 }}>{error}</p>}
    </div>
  );
}
