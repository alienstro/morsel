import { useEffect, useRef } from "react";
import { clock, type TranscriptLine } from "../../lib/api";

type Props = { lines: TranscriptLine[]; onHide: () => void };

/** The live transcript on the right. It follows the newest line unless the user scrolled up. */
export function TranscriptPanel({ lines, onHide }: Props) {
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
      <form className="ask-box" onSubmit={(e) => e.preventDefault()}>
        <label htmlFor="ask-meeting" className="sr-only">Ask about this meeting</label>
        <input id="ask-meeting" type="text" placeholder="Ask about this meeting" disabled title="Coming in a later version" />
        <button type="submit" className="btn btn-primary" disabled>Ask</button>
      </form>
    </aside>
  );
}
