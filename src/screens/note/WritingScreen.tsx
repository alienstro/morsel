import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { api, errorMessage, onNotesDone, onNotesFailed, onStatus, onTranscribeProgress, type MeetingDetail } from "../../lib/api";
import "./note.css";

type Phase = "transcribing" | "writing" | "failed" | "cancelled";
type Progress = { doneMs: number; totalMs: number };

const MINUTE_MS = 60_000;

/** "Transcribing: 12 of 60 min", or a short text before the first progress event. */
function transcribingText(progress: Progress | null): string {
  if (!progress || progress.totalMs <= 0) return "Transcribing the audio…";
  const done = Math.floor(progress.doneMs / MINUTE_MS);
  const total = Math.max(1, Math.ceil(progress.totalMs / MINUTE_MS));
  return `Transcribing: ${done} of ${total} min`;
}

export function WritingScreen() {
  const id = Number(useParams().id);
  const navigate = useNavigate();
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [phase, setPhase] = useState<Phase>("writing");
  const [message, setMessage] = useState("");
  const [progress, setProgress] = useState<Progress | null>(null);

  useEffect(() => {
    let alive = true;
    api.getMeeting(id).then((d) => {
      if (!alive) return;
      setDetail(d);
      if (d.summary.status === "done") navigate(`/note/${id}`, { replace: true });
      if (d.summary.status === "notes_failed") {
        setMessage(d.notesError ?? "");
        setPhase("failed");
      }
      if (d.summary.status === "notes_cancelled") setPhase("cancelled");
      if (d.summary.status === "transcribing") setPhase("transcribing");
    }).catch((e) => setMessage(errorMessage(e)));
    const unlisten = [
      onNotesDone((e) => e.meetingId === id && navigate(`/note/${id}`, { replace: true })),
      onNotesFailed((e) => {
        if (e.meetingId !== id) return;
        setMessage(e.message);
        setPhase("failed");
      }),
      onStatus((e) => {
        if (e.meetingId !== id) return;
        if (e.status === "notes_cancelled") setPhase("cancelled");
        if (e.status === "writing_notes") setPhase("writing");
      }),
      onTranscribeProgress((e) => e.meetingId === id && setProgress({ doneMs: e.doneMs, totalMs: e.totalMs })),
    ];
    return () => {
      alive = false;
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, [id, navigate]);

  const tryAgain = async () => {
    if (!detail) return;
    setPhase("writing");
    try {
      await api.writeNotes(id, detail.summary.templateId);
    } catch (e) {
      setMessage(errorMessage(e));
      setPhase("failed");
    }
  };

  return (
    <div className="page">
      <h1 className="serif page-title">{detail?.summary.title ?? "Writing notes"}</h1>
      {phase === "transcribing" && (
        <ol className="steps">
          <li>Your rough notes are saved.</li>
          <li aria-busy="true"><strong>{transcribingText(progress)}</strong></li>
          <li>Morsel writes the notes.</li>
        </ol>
      )}
      {phase === "writing" && (
        <>
          <ol className="steps">
            <li>Your rough notes are saved.</li>
            <li>The transcript is complete.</li>
            <li aria-busy="true"><strong>Morsel is writing the notes…</strong></li>
          </ol>
          <button type="button" className="btn" onClick={() => api.cancelNotes(id).catch(() => {})}>
            Cancel and keep my notes
          </button>
        </>
      )}
      {phase === "failed" && (
        <>
          <p role="alert" className="error-text">Morsel could not write the notes: {message || "unknown error"}</p>
          <p className="muted">Your rough notes and the transcript are saved.</p>
          <div className="row">
            <button type="button" className="btn btn-primary" onClick={tryAgain}>Try again</button>
            <Link to="/settings/models" className="btn">Open Settings</Link>
          </div>
        </>
      )}
      {phase === "cancelled" && (
        <>
          <p className="muted">Cancelled. Your rough notes and the transcript are saved.</p>
          <div className="rough serif">{detail?.userNotes || "(no rough notes)"}</div>
          <div className="row">
            <button type="button" className="btn btn-primary" onClick={tryAgain}>Write notes</button>
            <Link to={`/meeting/${id}`} className="btn">Open the meeting</Link>
          </div>
        </>
      )}
    </div>
  );
}
