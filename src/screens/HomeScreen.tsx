import { type FormEvent, useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";
import { api, errorMessage, needsSettings, onCalendarChanged, type CalendarEvent, type MeetingSummary, type Template } from "../lib/api";
import { eventLine } from "../lib/calendar";
import { clockTime, dayLabel } from "../lib/dates";
import "./library.css";
import "./note/note.css";

const LIVE = ["recording", "paused"];

/** Meetings in day groups, newest first: [["Today", [...]], ["Yesterday", [...]], ...]. */
function groupByDay(meetings: MeetingSummary[]): [string, MeetingSummary[]][] {
  const groups: [string, MeetingSummary[]][] = [];
  for (const m of meetings) {
    const day = dayLabel(m.startedAt);
    const last = groups[groups.length - 1];
    if (last && last[0] === day) last[1].push(m);
    else groups.push([day, [m]]);
  }
  return groups;
}

/** Home: the next calendar meeting (or a plain start), recent notes, and the Ask box. */
export function HomeScreen() {
  const navigate = useNavigate();
  const [templates, setTemplates] = useState<Template[]>([]);
  const [meetings, setMeetings] = useState<MeetingSummary[]>([]);
  const [templateId, setTemplateId] = useState(1);
  const [title, setTitle] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [question, setQuestion] = useState("");
  const askRef = useRef<HTMLInputElement>(null);
  const [next, setNext] = useState<CalendarEvent | null>(null);
  const [unscheduled, setUnscheduled] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const load = () => api.upcomingEvents().then((list) => setNext(list?.[0] ?? null)).catch(() => {});
    load();
    const off = onCalendarChanged(load);
    const timer = window.setInterval(() => {
      setNow(Date.now());
      load();
    }, 30_000);
    return () => {
      off.then((f) => f());
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        askRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const ask = (e: FormEvent) => {
    e.preventDefault();
    const q = question.trim();
    if (q) navigate(`/ask?q=${encodeURIComponent(q)}`);
  };

  useEffect(() => {
    api.listTemplates().then(setTemplates).catch((e) => setError(errorMessage(e)));
    api.listMeetings(8).then(setMeetings).catch(() => {});
  }, []);

  const event = unscheduled ? null : next;

  const start = async () => {
    setError(null);
    setStarting(true);
    try {
      const id = event
        ? await api.startMeeting(event.title, templateId, event.people)
        : await api.startMeeting(title.trim() || null, templateId);
      navigate(`/meeting/${id}`);
    } catch (e) {
      setError(errorMessage(e));
      setStarting(false);
    }
  };

  return (
    <div className="page">
      <h1 className="serif page-title">Good to see you.</h1>
      <section aria-labelledby="start" className="start-card">
        <h2 id="start" className="sr-only">{event ? "Next meeting" : "Start a meeting"}</h2>
        {event ? (
          <>
            <span className="tabular next-time">{clockTime(event.start)}</span>
            <div className="next-text">
              <div className="truncate next-title">{event.title}</div>
              <div className="truncate muted small">{eventLine(event, now)}</div>
            </div>
          </>
        ) : (
          <>
            <label htmlFor="title" className="sr-only">Meeting title</label>
            <input id="title" className="text-input" placeholder="Meeting title (optional)" value={title} onChange={(e) => setTitle(e.target.value)} />
          </>
        )}
        <label htmlFor="template" className="sr-only">Template</label>
        <select id="template" value={templateId} onChange={(e) => setTemplateId(Number(e.target.value))}>
          {templates.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
        </select>
        <button type="button" className="btn btn-primary" onClick={start} disabled={starting}>
          <span aria-hidden="true" className="dot" />
          Start notes
        </button>
      </section>
      {event && (
        <button type="button" className="btn btn-link next-skip" onClick={() => setUnscheduled(true)}>Start a note without the calendar</button>
      )}
      {error && <p role="alert" className="error-text">{error}{needsSettings(error) && <> <Link to="/settings/models">Open Settings</Link></>}</p>}
      <section aria-labelledby="earlier">
        <div className="section-head">
          <h2 id="earlier" className="section-title">Earlier notes</h2>
          {meetings.length > 0 && <Link to="/notes">View all</Link>}
        </div>
        {meetings.length === 0 && <p className="muted">Your notes show here after your first meeting.</p>}
        {groupByDay(meetings).map(([day, items]) => (
          <div key={day}>
            <h3 className="day-group">{day}</h3>
            <ul className="meeting-list" style={{ marginTop: 0 }}>
              {items.map((m) => (
                <li key={m.id}>
                  <Link to={LIVE.includes(m.status) ? `/meeting/${m.id}` : `/note/${m.id}`} className="meeting-row">
                    <span className="truncate" style={{ flexGrow: 1, fontWeight: 500 }}>{m.title}</span>
                    <span className="truncate muted small" title={m.templateName} style={{ flexShrink: 0, maxWidth: "40%" }}>{m.templateName}</span>
                    <span className="muted small tabular" style={{ width: 40, textAlign: "right" }}>{clockTime(m.startedAt)}</span>
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </section>
      <form className="home-ask" onSubmit={ask}>
        <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="var(--muted)" strokeWidth="2" strokeLinecap="round"><circle cx="11" cy="11" r="6" /><path d="M20 20l-4.5-4.5" /></svg>
        <label htmlFor="home-ask" className="sr-only">Ask about any meeting</label>
        <input ref={askRef} id="home-ask" type="text" placeholder="Ask about any meeting" value={question} onChange={(e) => setQuestion(e.target.value)} />
        <kbd>⌘K</kbd>
      </form>
    </div>
  );
}
