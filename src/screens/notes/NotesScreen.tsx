import { useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { api, errorMessage, type MeetingSummary, type NotesFilter, type Template } from "../../lib/api";
import { formatWhen, meetingLength } from "../../lib/dates";
import "../library.css";

const PAGE = 50;
const SEARCH_WAIT_MS = 250;
export const LIVE = ["recording", "paused"];

export function meetingPath(m: MeetingSummary): string {
  return LIVE.includes(m.status) ? `/meeting/${m.id}` : `/note/${m.id}`;
}

/** 4.1 All notes: search, filter by template and person, and open a note. */
export function NotesScreen() {
  const [query, setQuery] = useState("");
  const [sentQuery, setSentQuery] = useState("");
  const [templateId, setTemplateId] = useState<number | null>(null);
  const [person, setPerson] = useState<string | null>(null);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [people, setPeople] = useState<string[]>([]);
  const [items, setItems] = useState<MeetingSummary[]>([]);
  const [total, setTotal] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);

  useEffect(() => {
    api.listTemplates().then(setTemplates).catch(() => {});
    api.listPeople().then(setPeople).catch(() => {});
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => setSentQuery(query.trim()), SEARCH_WAIT_MS);
    return () => clearTimeout(timer);
  }, [query]);

  const filter = (offset: number): NotesFilter => ({
    query: sentQuery || null, templateId, person, limit: PAGE, offset,
  });

  useEffect(() => {
    const id = ++request.current;
    api.searchNotes(filter(0))
      .then((page) => {
        if (id !== request.current) return;
        setItems(page.items);
        setTotal(page.total);
        setLoaded(true);
        setError(null);
      })
      .catch((e) => setError(errorMessage(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sentQuery, templateId, person]);

  const showMore = async () => {
    try {
      const page = await api.searchNotes(filter(items.length));
      setItems((prev) => [...prev, ...page.items]);
      setTotal(page.total);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const filtered = Boolean(query.trim() || templateId !== null || person !== null);

  return (
    <div className="lib-page">
      <div className="lib-head">
        <h1 className="lib-title">Notes</h1>
        <div className="search-box">
          <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="var(--muted)" strokeWidth="2" strokeLinecap="round"><circle cx="11" cy="11" r="6" /><path d="M20 20l-4.5-4.5" /></svg>
          <label htmlFor="notes-q" className="sr-only">Search notes</label>
          <input id="notes-q" type="search" placeholder="Search notes" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <label htmlFor="notes-template" className="sr-only">Template</label>
        <select id="notes-template" className="lib-select" value={templateId ?? ""} onChange={(e) => setTemplateId(e.target.value ? Number(e.target.value) : null)}>
          <option value="">All templates</option>
          {templates.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
        </select>
        <label htmlFor="notes-person" className="sr-only">Person</label>
        <select id="notes-person" className="lib-select" value={person ?? ""} onChange={(e) => setPerson(e.target.value || null)}>
          <option value="">Anyone</option>
          {people.map((p) => <option key={p} value={p}>{p}</option>)}
        </select>
      </div>

      {error && <p role="alert" className="error-text">{error}</p>}

      {loaded && items.length === 0 ? (
        <p className="muted">{filtered ? "No note matches your search." : "Your notes show here after your first meeting."}</p>
      ) : (
        <div className="table-wrap">
          <table className="notes-table">
            <thead>
              <tr>
                <th scope="col">Meeting</th>
                <th scope="col" style={{ width: 120 }}>Template</th>
                <th scope="col" style={{ width: 180 }}>People</th>
                <th scope="col" style={{ width: 150 }}>Date</th>
                <th scope="col" className="num" style={{ width: 70 }} title="Time from start to stop, with the paused time">Length</th>
              </tr>
            </thead>
            <tbody>
              {items.map((m) => (
                <tr key={m.id}>
                  <td><Link to={meetingPath(m)}>{m.title}</Link></td>
                  <td>{m.templateName}</td>
                  <td>{m.people.length > 0 ? m.people.join(", ") : "—"}</td>
                  <td>{formatWhen(m.startedAt)}</td>
                  <td className="num">{meetingLength(m.startedAt, m.endedAt)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {loaded && items.length > 0 && (
        <div className="lib-foot">
          <span>{items.length} of {total} notes</span>
          {items.length < total && <button type="button" className="btn" onClick={showMore}>Show more</button>}
        </div>
      )}
    </div>
  );
}
