import { useEffect, useState } from "react";
import { api, errorMessage, type CalendarEvent } from "../lib/api";
import { eventWho } from "../lib/calendar";
import "./reminder.css";

function startsIn(e: CalendarEvent, now: number): string {
  const mins = Math.ceil((e.start - now) / 60_000);
  return mins > 0 ? `in ${mins} min` : "now";
}

/** The reminder window: one calendar meeting, with Start notes and Dismiss. */
export function ReminderApp() {
  const [event, setEvent] = useState<CalendarEvent | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.reminderEvent().then(setEvent).catch((e) => setError(errorMessage(e)));
  }, []);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.startFromReminder();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  if (!event) return error ? <p className="reminder-error">{error}</p> : null;
  const who = eventWho(event);

  return (
    <div role="alert" className="reminder">
      <div aria-hidden="true" className="reminder-logo serif">M</div>
      <div className="reminder-body">
        <div className="reminder-head">
          <span className="reminder-title truncate">{event.title} {startsIn(event, Date.now())}</span>
          <span className="reminder-now">now</span>
        </div>
        {who && <div className="reminder-who truncate">{who}</div>}
        {error && <div className="reminder-error truncate">{error}</div>}
        <div className="reminder-actions">
          <button type="button" className="btn btn-primary" onClick={start} disabled={busy}>Start notes</button>
          <button type="button" className="btn" onClick={() => api.dismissReminder()}>Dismiss</button>
        </div>
      </div>
    </div>
  );
}
