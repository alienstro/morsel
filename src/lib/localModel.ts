import { useEffect, useState } from "react";
import { api, errorMessage, onModelDownload, onNotesModelDownload } from "./api";

export type ModelInfo = { name: string; size: string; about: string };

/** The on-device speech model. Whisper Turbo knows most languages, Tagalog too. */
export const LOCAL_MODEL: ModelInfo = { name: "Whisper Turbo", size: "about 1 GB", about: "Most languages, Tagalog too" };

/** The on-device notes model. It writes the notes and answers Ask on this Mac. */
export const NOTES_MODEL: ModelInfo = {
  name: "MiniCPM-V-4",
  size: "2.96 GB",
  about: "Writes the notes and answers questions on this Mac. No meeting text leaves it.",
};

export type Progress = { received: number; total: number };

type ModelSource = {
  status: () => Promise<boolean>;
  download: () => Promise<void>;
  onProgress: (h: (p: Progress) => void) => Promise<() => void>;
};

const WHISPER: ModelSource = { status: api.modelStatus, download: api.downloadModel, onProgress: onModelDownload };
const NOTES: ModelSource = { status: api.notesModelStatus, download: api.downloadNotesModel, onProgress: onNotesModelDownload };

/** The download state of one on-device model. */
function useModelDownload(source: ModelSource) {
  const [ready, setReady] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    source.status().then(setReady).catch(() => {});
    const off = source.onProgress(setProgress);
    return () => {
      off.then((f) => f());
    };
  }, [source]);

  const download = async () => {
    setError(null);
    setDownloading(true);
    setProgress({ received: 0, total: 0 });
    try {
      await source.download();
      setReady(true);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setDownloading(false);
      setProgress(null);
    }
  };

  return { ready, downloading, progress, error, download };
}

export type ModelDownload = ReturnType<typeof useModelDownload>;

/** The download state of the on-device speech model. */
export const useLocalModel = () => useModelDownload(WHISPER);

/** The download state of the on-device notes model. */
export const useNotesModel = () => useModelDownload(NOTES);

const MB = 1_000_000;

/** "Ready", "Downloading, 384 of 600 MB", or null when the model still needs a download. */
export function downloadStateText(model: ModelDownload): string | null {
  if (model.ready) return "Ready";
  if (!model.downloading) return null;
  const p = model.progress;
  return p && p.total > 0 ? `Downloading, ${Math.floor(p.received / MB)} of ${Math.floor(p.total / MB)} MB` : "Downloading…";
}
