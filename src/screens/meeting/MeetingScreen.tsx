import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import {
  api, clock, errorMessage, onLevels, onStatus, onTranscript,
  type MeetingDetail, type MeetingStatus, type MeetingSummary, type TranscriptLine,
} from "../../lib/api";
import { Notepad } from "./Notepad";
import { TranscriptPanel } from "./TranscriptPanel";
import "./meeting.css";

const SAVE_DELAY_MS = 2000;
const LIVE: MeetingStatus[] = ["recording", "paused"];

function Bars({ level }: { level: number }) {
  const heights = [0.5, 1, 0.7].map((f) => Math.max(2, Math.min(10, Math.round(level * 40 * f))));
  return (
    <span aria-hidden="true" className="bars">
      {heights.map((h, i) => <span key={i} style={{ height: h }} />)}
    </span>
  );
}

export function MeetingScreen() {
  const id = Number(useParams().id);
  const navigate = useNavigate();
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [meetings, setMeetings] = useState<MeetingSummary[]>([]);
  const [lines, setLines] = useState<TranscriptLine[]>([]);
  const [status, setStatus] = useState<MeetingStatus>("recording");
  const [warning, setWarning] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [levels, setLevels] = useState({ you: 0, others: 0 });
  const [showTranscript, setShowTranscript] = useState(true);
  const [now, setNow] = useState(Date.now());
  const notesRef = useRef("");
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let alive = true;
    api.getMeeting(id).then((d) => {
      if (!alive) return;
      setDetail(d);
      setLines(d.lines);
      setStatus(d.summary.status);
      notesRef.current = d.userNotes;
    }).catch((e) => setError(errorMessage(e)));
    api.listMeetings(8).then((m) => alive && setMeetings(m)).catch(() => {});
    const unlisten = [
      onTranscript((line) => line.meetingId === id && setLines((prev) => [...prev, line])),
      onStatus((e) => {
        if (e.meetingId !== id) return;
        setStatus(e.status);
        if (e.warning) setWarning(e.warning);
      }),
      onLevels((l) => setLevels(l)),
    ];
    return () => {
      alive = false;
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, [id]);

  useEffect(() => {
    if (status !== "recording") return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [status]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "t") {
        e.preventDefault();
        setShowTranscript((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => () => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
  }, []);

  const saveNow = useCallback(async () => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = null;
    await api.saveUserNotes(id, notesRef.current);
  }, [id]);

  const onNotesChange = (text: string) => {
    notesRef.current = text;
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      saveNow().catch((e) => setError(errorMessage(e)));
    }, SAVE_DELAY_MS);
  };

  const run = async (action: () => Promise<void>) => {
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const stopAndWrite = () => run(async () => {
    await saveNow();
    if (LIVE.includes(status)) await api.stopMeeting();
    await api.writeNotes(id, detail!.summary.templateId);
    navigate(`/meeting/${id}/writing`);
  });

  if (!detail) {
    return <p className="muted" style={{ padding: 32 }}>{error ?? "Loading the meeting…"}</p>;
  }

  const live = LIVE.includes(status);
  const elapsed = clock(Math.max(0, now - detail.summary.startedAt));

  return (
    <div className="meeting">
      <div className="titlebar" data-tauri-drag-region />
      <div className="meeting-body">
        <nav aria-label="Meetings" className="meeting-nav">
          <Link to="/" className="brand meeting-brand">Morsel</Link>
          <div className="nav-group">
            <div className="nav-label">Meetings</div>
            {meetings.map((m) => (
              <Link key={m.id} to={m.id === id ? `/meeting/${m.id}` : `/note/${m.id}`} aria-current={m.id === id ? "page" : undefined} className="nav-link">
                {m.id === id && live && <span aria-hidden="true" className="dot" />}
                <span className="truncate">{m.title}</span>
              </Link>
            ))}
          </div>
        </nav>

        <main className="meeting-main">
          <div className="meeting-card">
            <div className="toolbar">
              <div role="status" className="rec-status">
                {live ? (
                  <>
                    <span aria-hidden="true" className="dot" />
                    <span className="rec-label">{status === "paused" ? "Paused" : "Recording"}</span>
                    <span className="muted tabular">{elapsed}</span>
                    <span className="muted level">You <Bars level={levels.you} /></span>
                    <span className="muted level">Others <Bars level={levels.others} /></span>
                  </>
                ) : (
                  <span className="muted">This meeting is not recording.</span>
                )}
                {warning && <span className="warning">{warning}</span>}
              </div>
              <span style={{ flexGrow: 1 }} />
              {!showTranscript && (
                <button type="button" className="btn" aria-expanded="false" aria-controls="tx-panel" onClick={() => setShowTranscript(true)}>
                  Show transcript <kbd>⌘T</kbd>
                </button>
              )}
              {live && (
                <button type="button" className="btn" onClick={() => run(status === "paused" ? api.resumeMeeting : api.pauseMeeting)}>
                  {status === "paused" ? "Resume" : "Pause"}
                </button>
              )}
              <button type="button" className="btn btn-primary" onClick={stopAndWrite}>
                {live ? "Stop and write notes" : "Write notes"}
              </button>
            </div>
            {error && <p role="alert" className="error-text">{error}</p>}
            <article className="meeting-article">
              <h1 className="serif meeting-title">{detail.summary.title}</h1>
              <div className="meeting-meta muted">
                <span>{new Date(detail.summary.startedAt).toLocaleString()}</span>
                <span className="chip">{detail.summary.templateName}</span>
              </div>
              <Notepad initial={detail.userNotes} onChange={onNotesChange} />
              <p className="muted small">Write rough points. When you stop, Morsel adds the details from the transcript.</p>
            </article>
          </div>
        </main>

        {showTranscript && <TranscriptPanel meetingId={id} lines={lines} onHide={() => setShowTranscript(false)} />}
      </div>
    </div>
  );
}
