import { save } from "@tauri-apps/plugin-dialog";
import { type ReactNode, useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { ConfirmDialog } from "../../app/ConfirmDialog";
import { api, clock, errorMessage, type MeetingDetail, type Template } from "../../lib/api";
import { markYours, type NotePart } from "../../lib/markYours";
import { LIVE } from "../notes/NotesScreen";
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
  const [confirming, setConfirming] = useState(false);
  const [audioBytes, setAudioBytes] = useState<number | null>(null);
  const [confirmingAudio, setConfirmingAudio] = useState(false);

  useEffect(() => {
    let alive = true;
    api.getMeeting(id).then((d) => {
      if (!alive) return;
      // The after job still transcribes this meeting: the writing page shows its progress.
      if (d.summary.status === "transcribing") {
        navigate(`/meeting/${id}/writing`, { replace: true });
        return;
      }
      // A live meeting has no notes yet: the meeting screen stops it first (#18).
      if (LIVE.includes(d.summary.status)) {
        navigate(`/meeting/${id}`, { replace: true });
        return;
      }
      setDetail(d);
      setTemplateId(d.summary.templateId);
    }).catch((e) => setInfo(errorMessage(e)));
    api.listTemplates().then(setTemplates).catch(() => {});
    api.audioSize(id).then(setAudioBytes).catch(() => {});
    return () => {
      alive = false;
    };
  }, [id, navigate]);

  const deleteAudio = async () => {
    setConfirmingAudio(false);
    try {
      await api.deleteAudio(id);
      setAudioBytes(null);
      setInfo("The audio is deleted. The notes and the transcript stay.");
    } catch (e) {
      setInfo(errorMessage(e));
    }
  };

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

  const remove = async () => {
    setConfirming(false);
    try {
      await api.deleteMeeting(id);
      navigate("/notes");
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
        <button type="button" className="icon-btn-light" aria-label="Delete note" onClick={() => setConfirming(true)}>
          <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" /></svg>
        </button>
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
      {audioBytes !== null && (
        <div className="row">
          <button type="button" className="btn" onClick={() => setConfirmingAudio(true)}>
            Delete audio ({Math.max(1, Math.round(audioBytes / 1_000_000))} MB)
          </button>
        </div>
      )}
      {confirmingAudio && (
        <ConfirmDialog
          title="Delete the audio?"
          body="The recording of this meeting goes away. The notes and the transcript stay. You cannot undo this."
          confirmLabel="Delete audio"
          onConfirm={deleteAudio}
          onCancel={() => setConfirmingAudio(false)}
        />
      )}
      {confirming && (
        <ConfirmDialog
          title="Delete this note?"
          body="The note, your rough notes and the transcript go away. You cannot undo this."
          confirmLabel="Delete note"
          onConfirm={remove}
          onCancel={() => setConfirming(false)}
        />
      )}
    </div>
  );
}
