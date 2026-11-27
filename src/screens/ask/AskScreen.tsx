import { type FormEvent, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { api, errorMessage, needsSettings, type AskAnswer, type AskRange } from "../../lib/api";
import { dayMonth } from "../../lib/dates";
import "../library.css";
import { AnswerText } from "./AnswerText";

const HISTORY = 20;

/** 4.2 Ask Morsel: a question across meetings, with the meetings that the answer comes from. */
export function AskScreen() {
  const params = useParams();
  const savedId = params.id ? Number(params.id) : null;
  const [search] = useSearchParams();
  const navigate = useNavigate();
  const [question, setQuestion] = useState("");
  const [range, setRange] = useState<AskRange>("all");
  const [answer, setAnswer] = useState<AskAnswer | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [earlier, setEarlier] = useState<AskAnswer[]>([]);
  const [error, setError] = useState<string | null>(null);
  const askedFromUrl = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  // A new question bumps this, so an older answer that arrives late goes only to the history.
  const turn = useRef(0);

  const send = async (text: string, scope: AskRange) => {
    const q = text.trim();
    if (!q) return;
    setError(null);
    setPending(q);
    const mine = ++turn.current;
    try {
      const result = await api.ask(q, scope, null);
      if (result.id !== null) setEarlier((prev) => [result, ...prev.filter((a) => a.id !== result.id)].slice(0, HISTORY));
      if (mine !== turn.current) return;
      setAnswer(result);
      setQuestion("");
      if (result.id !== null) navigate(`/ask/${result.id}`, { replace: true });
    } catch (e) {
      if (mine === turn.current) setError(errorMessage(e));
    } finally {
      if (mine === turn.current) setPending(null);
    }
  };

  const clear = () => {
    turn.current++;
    setAnswer(null);
    setPending(null);
    setQuestion("");
    setError(null);
  };

  const newQuestion = () => {
    clear();
    if (savedId !== null) navigate("/ask");
    input.current?.focus();
  };

  useEffect(() => {
    api.listAsks(HISTORY).then(setEarlier).catch(() => {});
  }, []);

  useEffect(() => {
    const q = search.get("q");
    if (q && !askedFromUrl.current && savedId === null) {
      askedFromUrl.current = true;
      void send(q, "all");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search]);

  useEffect(() => {
    // The sidebar link opens /ask: show an empty page, not the last answer.
    if (savedId === null) {
      if (answer || error) clear();
      return;
    }
    if (answer?.id === savedId) return;
    // An earlier question opens: an answer that is still loading goes only to the history.
    turn.current++;
    setPending(null);
    setError(null);
    let live = true;
    api.getAsk(savedId).then((a) => live && setAnswer(a)).catch((e) => live && setError(errorMessage(e)));
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [savedId]);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    void send(question, range);
  };

  const shown = pending ?? answer?.question ?? null;

  return (
    <div className="ask-layout">
      {earlier.length > 0 && (
        <nav aria-label="Earlier questions" className="ask-history">
          <div className="nav-label">Earlier questions</div>
          {earlier.map((a) => (
            <Link key={a.id} to={`/ask/${a.id}`} aria-current={a.id === savedId ? "page" : undefined} title={a.question}>{a.question}</Link>
          ))}
        </nav>
      )}
      <div className="ask-main">
        <div className="ask-thread">
          <h1>Ask Morsel</h1>
          {shown && <button type="button" className="btn" style={{ alignSelf: "flex-start" }} onClick={newQuestion}>New question</button>}
          {!shown && <p className="muted">Ask a question about your meetings. Morsel answers from your notes and transcripts, and shows where each answer comes from.</p>}
          {shown && <div className="question-bubble">{shown}</div>}
          {pending && <p role="status" className="muted">Reading your meetings…</p>}
          {!pending && answer && (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              <AnswerText text={answer.answer} className="answer-text" />
              {answer.sources.length > 0 && (
                <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                  <div className="muted" style={{ fontSize: 12 }}>From {answer.sources.length} {answer.sources.length === 1 ? "meeting" : "meetings"}</div>
                  <div className="sources">
                    {answer.sources.map((s) => (
                      <Link key={s.meetingId} to={`/note/${s.meetingId}`} className="source-link">
                        <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round"><path d="M6 3h9l4 4v14H6z" /></svg>
                        {s.title}, {dayMonth(s.startedAt)}
                      </Link>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
          {error && <p role="alert" className="error-text">{error}{needsSettings(error) && <> <Link to="/settings/models">Open Settings</Link></>}</p>}
        </div>
        <form className="ask-form" onSubmit={submit}>
          <label htmlFor="ask-q" className="sr-only">Ask about your meetings</label>
          <input ref={input} id="ask-q" type="text" placeholder="Ask about your meetings" value={question} onChange={(e) => setQuestion(e.target.value)} />
          <label htmlFor="ask-range" className="sr-only">Time range</label>
          <select id="ask-range" value={range} onChange={(e) => setRange(e.target.value as AskRange)}>
            <option value="all">All meetings</option>
            <option value="week">This week</option>
            <option value="month">This month</option>
          </select>
          <button type="submit" className="btn btn-primary" disabled={!question.trim() || pending !== null}>Ask</button>
        </form>
      </div>
    </div>
  );
}
