import { useEffect, useState } from "react";
import { api, errorMessage, onModelDownload, onNotesModelDownload, onSearchModelDownload, onSearchProgress } from "./api";

export type ModelInfo = { name: string; size: string; about: string };

/** The on-device speech model. Whisper Turbo knows most languages, Tagalog too. */
export const LOCAL_MODEL: ModelInfo = { name: "Whisper Turbo", size: "about 1 GB", about: "Most languages, Tagalog too" };

/** The on-device notes model. It writes the notes and answers Ask on this Mac. */
export const NOTES_MODEL: ModelInfo = {
  name: "MiniCPM-V-4",
  size: "2.96 GB",
  about: "Writes the notes and answers questions on this Mac. No meeting text leaves it.",
};

/** The on-device search model. It finds the parts of the meetings that Ask reads. */
export const SEARCH_MODEL: ModelInfo = {
  name: "EmbeddingGemma 2 search",
  size: "310 MB",
  about: "Finds the parts of your meetings that answer a question in Ask, on this Mac.",
};

/** The notes model and the search model together. */
export const LOCAL_DOWNLOAD_TOTAL = "3.27 GB";

export type Progress = { received: number; total: number };

type ModelSource = {
  status: () => Promise<boolean>;
  download: () => Promise<void>;
  onProgress: (h: (p: Progress) => void) => Promise<() => void>;
};

const WHISPER: ModelSource = { status: api.modelStatus, download: api.downloadModel, onProgress: onModelDownload };
const NOTES: ModelSource = { status: api.notesModelStatus, download: api.downloadNotesModel, onProgress: onNotesModelDownload };
const SEARCH: ModelSource = { status: api.searchModelStatus, download: api.downloadSearchModel, onProgress: onSearchModelDownload };

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

  /** Downloads the model. Resolves to true when the model is ready. */
  const download = async (): Promise<boolean> => {
    setError(null);
    setDownloading(true);
    setProgress({ received: 0, total: 0 });
    try {
      await source.download();
      setReady(true);
      return true;
    } catch (e) {
      setError(errorMessage(e));
      return false;
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

/** The download state of the on-device search model. */
export const useSearchModel = () => useModelDownload(SEARCH);

export type IndexProgress = { done: number; total: number };

/** The search index while it runs, or null when it is idle or complete. */
export function useSearchIndex(): IndexProgress | null {
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  useEffect(() => {
    const off = onSearchProgress((p) => setProgress(p.done < p.total ? p : null));
    return () => {
      off.then((f) => f());
    };
  }, []);
  return progress;
}

const MB = 1_000_000;

/** "Ready", "Downloading, 384 of 600 MB", or null when the model still needs a download. */
export function downloadStateText(model: ModelDownload): string | null {
  if (model.ready) return "Ready";
  if (!model.downloading) return null;
  const p = model.progress;
  return p && p.total > 0 ? `Downloading, ${Math.floor(p.received / MB)} of ${Math.floor(p.total / MB)} MB` : "Downloading…";
}
