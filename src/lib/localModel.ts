import { useEffect, useState, useSyncExternalStore } from "react";
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

type DownloadState = { ready: boolean; downloading: boolean; progress: Progress | null; error: string | null };

const IDLE: DownloadState = { ready: false, downloading: false, progress: null, error: null };

/** The download state of one model, outside React, so that it survives a screen change (#21). */
function modelStore(source: ModelSource) {
  let state = IDLE;
  let running: Promise<boolean> | null = null;
  const listeners = new Set<() => void>();
  const set = (patch: Partial<DownloadState>) => {
    state = { ...state, ...patch };
    listeners.forEach((l) => l());
  };
  const run = async (): Promise<boolean> => {
    set({ error: null, downloading: true, progress: { received: 0, total: 0 } });
    // The store listens, not the screen, so that no progress is lost while no screen shows it.
    const off = source.onProgress((progress) => set({ progress }));
    try {
      await source.download();
      set({ ready: true });
      return true;
    } catch (e) {
      set({ error: errorMessage(e) });
      return false;
    } finally {
      off.then((f) => f());
      set({ downloading: false, progress: null });
    }
  };
  return {
    source,
    set,
    get: () => state,
    subscribe: (l: () => void) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    /** Downloads the model, or joins the download that runs. Resolves to true when the model is ready. */
    download: (): Promise<boolean> => {
      if (running) return running;
      const p = run().finally(() => {
        if (running === p) running = null;
      });
      return (running = p);
    },
    reset: () => {
      running = null;
      set(IDLE);
    },
  };
}

type ModelStore = ReturnType<typeof modelStore>;

const WHISPER = modelStore({ status: api.modelStatus, download: api.downloadModel, onProgress: onModelDownload });
const NOTES = modelStore({ status: api.notesModelStatus, download: api.downloadNotesModel, onProgress: onNotesModelDownload });
const SEARCH = modelStore({ status: api.searchModelStatus, download: api.downloadSearchModel, onProgress: onSearchModelDownload });

/** Clears the download state of all models. Tests call it, because the state outlives each render. */
export function resetModelDownloads() {
  for (const store of [WHISPER, NOTES, SEARCH]) store.reset();
}

/** The download state of one on-device model. */
function useModelDownload(store: ModelStore) {
  const state = useSyncExternalStore(store.subscribe, store.get);

  useEffect(() => {
    // A status call that started before the download ended can answer false, so only true counts.
    store.source.status().then((ready) => ready && store.set({ ready })).catch(() => {});
  }, [store]);

  return { ...state, download: store.download };
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
