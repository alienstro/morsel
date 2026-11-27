import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import { ConfirmDialog } from "../../app/ConfirmDialog";
import { api, errorMessage, type DataInfo } from "../../lib/api";
import "../library.css";
import "../note/note.css";

/** "40 KB", "6 MB", "1.5 GB". */
export function formatBytes(bytes: number): string {
  const kb = bytes / 1024;
  if (kb < 1024) return `${Math.max(1, Math.round(kb))} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${Math.round(mb)} MB`;
  return `${Math.round((mb / 1024) * 10) / 10} GB`;
}

/** 5.2 Privacy and data: the data folder, export all notes, and delete all data. */
export function SettingsPrivacyScreen() {
  const [info, setInfo] = useState<DataInfo | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  const load = () => api.dataInfo().then(setInfo).catch((e) => setError(errorMessage(e)));
  useEffect(() => {
    void load();
  }, []);

  const run = async (action: () => Promise<string | null>) => {
    setError(null);
    setMessage(null);
    try {
      const done = await action();
      if (done) setMessage(done);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const exportAll = () =>
    run(async () => {
      const dir = await open({ directory: true, multiple: false, title: "Choose a folder for the notes" });
      if (typeof dir !== "string") return null;
      const count = await api.exportAll(dir);
      return count === 1 ? "Exported 1 note." : `Exported ${count} notes.`;
    });

  const deleteAll = () => {
    setConfirming(false);
    void run(async () => {
      await api.deleteAllData();
      await load();
      return "All data is deleted.";
    });
  };

  return (
    <div className="page">
      <h1 className="serif page-title">Privacy and data</h1>
      <div className="settings-rows">
        <div className="settings-row">
          <span>Morsel keeps the audio of each meeting on this Mac until you delete it. Delete it from the note, or delete all data below.</span>
        </div>
        <div className="settings-row">
          <span className="grow">
            Data folder
            <span className="muted small block truncate">{info ? `${info.path}, ${info.notes} notes, ${formatBytes(info.bytes)}` : "…"}</span>
          </span>
          <button type="button" className="btn" disabled={!info} onClick={() => info && run(async () => { await revealItemInDir(info.path); return null; })}>Show in Finder</button>
        </div>
        <div className="settings-row">
          <span className="grow">Export all notes as Markdown</span>
          <button type="button" className="btn" onClick={exportAll}>Export</button>
        </div>
        <div className="settings-row">
          <span className="grow">
            Delete all notes, transcripts, audio and keys
            <span className="muted small block">You cannot undo this.</span>
          </span>
          <button type="button" className="btn btn-danger" onClick={() => setConfirming(true)}>Delete all data</button>
        </div>
      </div>
      {message && <p role="status" className="muted">{message}</p>}
      {error && <p role="alert" className="error-text">{error}</p>}
      {confirming && (
        <ConfirmDialog
          title="Delete all data?"
          body="Morsel deletes every note, rough note, transcript, meeting audio, question and setting, and the API keys in the Keychain. You cannot undo this."
          confirmLabel="Delete all data"
          onConfirm={deleteAll}
          onCancel={() => setConfirming(false)}
        />
      )}
    </div>
  );
}
