import { type FormEvent, useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";
import { api, errorMessage, type MeetingSummary, type Template } from "../lib/api";
import "./library.css";
import "./note/note.css";

const LIVE = ["recording", "paused"];

/** Basic Home for Plan 1: start an unscheduled meeting and open recent notes. The calendar card comes in Plan 3. */
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
    api.listMeetings(5).then(setMeetings).catch(() => {});
  }, []);

  const start = async () => {
    setError(null);
    setStarting(true);
    try {
      const id = await api.startMeeting(title.trim() || null, templateId);
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
        <h2 id="start" className="sr-only">Start a meeting</h2>
        <label htmlFor="title" className="sr-only">Meeting title</label>
        <input id="title" className="text-input" placeholder="Meeting title (optional)" value={title} onChange={(e) => setTitle(e.target.value)} />
        <label htmlFor="template" className="sr-only">Template</label>
        <select id="template" value={templateId} onChange={(e) => setTemplateId(Number(e.target.value))}>
          {templates.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
        </select>
        <button type="button" className="btn btn-primary" onClick={start} disabled={starting}>
          <span aria-hidden="true" className="dot" />
          Start notes
        </button>
      </section>
      {error && <p role="alert" className="error-text">{error}</p>}
      <section aria-labelledby="earlier">
        <h2 id="earlier" className="section-title">Earlier notes</h2>
        {meetings.length === 0 && <p className="muted">Your notes show here after your first meeting.</p>}
        <ul className="meeting-list">
          {meetings.map((m) => (
            <li key={m.id}>
              <Link to={LIVE.includes(m.status) ? `/meeting/${m.id}` : `/note/${m.id}`} className="meeting-row">
                <span className="truncate" style={{ flexGrow: 1, fontWeight: 500 }}>{m.title}</span>
                <span className="muted small">{m.templateName}</span>
              </Link>
            </li>
          ))}
        </ul>
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
