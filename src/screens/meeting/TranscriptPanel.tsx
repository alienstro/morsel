import { type FormEvent, useEffect, useRef, useState } from "react";
import { api, clock, errorMessage, type TranscriptLine } from "../../lib/api";
import { AnswerText } from "../ask/AnswerText";
import "../library.css";

type Props = { meetingId: number; lines: TranscriptLine[]; onHide: () => void };

/** The live transcript on the right. It follows the newest line unless the user scrolled up. */
export function TranscriptPanel({ meetingId, lines, onHide }: Props) {
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  const listRef = useRef<HTMLOListElement>(null);
  const followRef = useRef(true);

  useEffect(() => {
    const list = listRef.current;
    if (list && followRef.current) list.scrollTop = list.scrollHeight;
  }, [lines.length]);

  const onScroll = () => {
    const list = listRef.current;
    if (list) followRef.current = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
  };

  const ask = async (e: FormEvent) => {
    e.preventDefault();
    const q = question.trim();
    if (!q) return;
    setAsking(true);
    try {
      const result = await api.ask(q, "all", meetingId);
      setAnswer(result.answer);
      setQuestion("");
    } catch (err) {
      setAnswer(errorMessage(err));
    } finally {
      setAsking(false);
    }
  };

  return (
    <aside id="tx-panel" aria-label="Transcript" className="tx-panel">
      <div className="tx-head">
        <h2 className="tx-title">Transcript</h2>
        <span className="muted small">On this Mac</span>
        <button type="button" className="btn-ghost" aria-label="Hide transcript" aria-expanded="true" aria-controls="tx-panel" title="Hide transcript (⌘T)" onClick={onHide}>
          <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round"><path d="M9 6l6 6-6 6" /></svg>
          Hide
        </button>
      </div>
      <ol ref={listRef} onScroll={onScroll} className="tx-lines">
        {lines.length === 0 && <li className="muted small">Lines show here a few seconds after people speak.</li>}
        {lines.map((line, i) => (
          <li key={`${line.speaker}-${line.startMs}-${i}`} className={line.speaker === "you" ? "tx-line tx-you" : "tx-line"}>
            <span className="muted small">{line.speaker === "you" ? "You" : "Others"}, {clock(line.startMs)}</span>
            <span>{line.text}</span>
          </li>
        ))}
      </ol>
      {(asking || answer) && (
        <div role="status" className="panel-answer">
          {asking ? <p className="muted">Reading this meeting…</p> : <AnswerText text={answer ?? ""} />}
        </div>
      )}
      <form className="ask-box" onSubmit={ask}>
        <label htmlFor="ask-meeting" className="sr-only">Ask about this meeting</label>
        <input id="ask-meeting" type="text" placeholder="Ask about this meeting" value={question} onChange={(e) => setQuestion(e.target.value)} />
        <button type="submit" className="btn btn-primary" disabled={!question.trim() || asking}>Ask</button>
      </form>
    </aside>
  );
}
