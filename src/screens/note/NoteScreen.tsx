import { save } from "@tauri-apps/plugin-dialog";
import { type ReactNode, useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { api, clock, errorMessage, type MeetingDetail, type Template } from "../../lib/api";
import { markYours, type NotePart } from "../../lib/markYours";
import "./note.css";

const HEADING = /^#{1,3}\s+(.*)$/;
const TASK = /^\s*[-*+]\s+\[([ xX])\]\s+(.*)$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;

/** Renders the note Markdown that the notes model writes: headings, tasks, bullets, and paragraphs. */
function NoteBody({ parts }: { parts: NotePart[] }) {
  const blocks: ReactNode[] = [];
  let items: ReactNode[] = [];
  const flush = () => {
    if (items.length) blocks.push(<ul key={`ul-${blocks.length}`} className="note-list">{items}</ul>);
    items = [];
  };
  parts.forEach((part, i) => {
    const cls = part.yours ? undefined : "morsel";
    const heading = part.text.match(HEADING);
    const task = part.text.match(TASK);
    const bullet = part.text.match(BULLET);
    if (heading) {
      flush();
      blocks.push(<h2 key={i} className="note-heading">{heading[1]}</h2>);
    } else if (task) {
      items.push(
        <li key={i} className={cls ? `task ${cls}` : "task"}>
          <input type="checkbox" disabled defaultChecked={task[1] !== " "} aria-label={task[2]} />
          <span>{task[2]}</span>
        </li>,
      );
    } else if (bullet) {
      items.push(<li key={i} className={cls}><span>{bullet[1]}</span></li>);
    } else if (part.text.trim()) {
      flush();
      blocks.push(<p key={i} className={cls}>{part.text}</p>);
    }
  });
  flush();
  return <div className="note-body serif">{blocks}</div>;
}

export function NoteScreen() {
  const id = Number(useParams().id);
  const navigate = useNavigate();
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [templateId, setTemplateId] = useState<number | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  useEffect(() => {
    api.getMeeting(id).then((d) => {
      setDetail(d);
      setTemplateId(d.summary.templateId);
    }).catch((e) => setInfo(errorMessage(e)));
    api.listTemplates().then(setTemplates).catch(() => {});
  }, [id]);

  if (!detail) return <p className="muted page">{info ?? "Loading the note…"}</p>;

  const note = detail.note;
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(note ?? "");
      setInfo("Copied.");
    } catch (e) {
      setInfo(errorMessage(e));
    }
  };
  const exportMarkdown = async () => {
    try {
      const path = await save({ defaultPath: `${detail.summary.title}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
      if (!path) return;
      await api.exportNote(id, path);
      setInfo("Exported.");
    } catch (e) {
      setInfo(errorMessage(e));
    }
  };
  const writeAgain = async () => {
    try {
      await api.writeNotes(id, templateId ?? detail.summary.templateId);
      navigate(`/meeting/${id}/writing`);
    } catch (e) {
      setInfo(errorMessage(e));
    }
  };

  return (
    <div className="page">
      <h1 className="serif page-title">{detail.summary.title}</h1>
      <div className="row muted small">
        <span>{new Date(detail.summary.startedAt).toLocaleString()}</span>
        <span className="chip">{detail.summary.templateName}</span>
      </div>
      <div className="row">
        <button type="button" className="btn" onClick={copy} disabled={!note}>Copy</button>
        <button type="button" className="btn" onClick={exportMarkdown} disabled={!note}>Export as Markdown</button>
        <span style={{ flexGrow: 1 }} />
        <label htmlFor="template" className="sr-only">Template</label>
        <select id="template" value={templateId ?? ""} onChange={(e) => setTemplateId(Number(e.target.value))}>
          {templates.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
        </select>
        <button type="button" className="btn btn-primary" onClick={writeAgain}>{note ? "Write again" : "Write notes"}</button>
      </div>
      {info && <p role="status" className="muted small">{info}</p>}
      {note ? (
        <>
          <p className="legend small">
            <span>Your words</span>
            <span className="morsel">Added by Morsel</span>
          </p>
          <NoteBody parts={markYours(note, detail.userNotes)} />
        </>
      ) : (
        <p className="muted">This meeting has no notes yet.</p>
      )}
      <details className="transcript">
        <summary>Transcript ({detail.lines.length} lines)</summary>
        <ol className="tx-lines">
          {detail.lines.map((l, i) => (
            <li key={i} className={l.speaker === "you" ? "tx-line tx-you" : "tx-line"}>
              <span className="muted small">{l.speaker === "you" ? "You" : "Others"}, {clock(l.startMs)}</span>
              <span>{l.text}</span>
            </li>
          ))}
        </ol>
      </details>
    </div>
  );
}
