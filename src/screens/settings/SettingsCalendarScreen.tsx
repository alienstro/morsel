import { useEffect, useState } from "react";
import { api, errorMessage, type Settings } from "../../lib/api";
import { CalendarPanel } from "../calendar/CalendarPanel";
import "../onboarding/onboarding.css";

/** Settings: the calendar link and the meeting reminder. */
export function SettingsCalendarScreen() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings().then(setSettings).catch((e) => setError(errorMessage(e)));
  }, []);

  const toggleReminder = async () => {
    if (!settings) return;
    const next = { ...settings, remindBeforeMeeting: !settings.remindBeforeMeeting };
    setSettings(next);
    try {
      await api.saveSettings(next);
    } catch (e) {
      setSettings(settings);
      setError(errorMessage(e));
    }
  };

  return (
    <div className="page">
      <h1 className="serif page-title">Calendar</h1>
      <section className="settings-section" aria-label="Calendar link">
        <p className="muted small" style={{ margin: 0 }}>
          Morsel uses your Composio account to read the title, the time, the video link and the people of each event. It never changes your calendar.
        </p>
        <CalendarPanel />
      </section>
      {settings && (
        <section className="settings-section" aria-label="Reminder">
          <label className="row">
            <input type="checkbox" role="switch" checked={settings.remindBeforeMeeting} onChange={toggleReminder} />
            Remind me 1 minute before a meeting
          </label>
        </section>
      )}
      {error && <p role="alert" className="error-text small">{error}</p>}
    </div>
  );
}
