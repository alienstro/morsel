import { useEffect, useState } from "react";
import { api, errorMessage, onModelDownload } from "./api";

/** The on-device speech model. Whisper Turbo knows most languages, Tagalog too. */
export const LOCAL_MODEL = { name: "Whisper Turbo", size: "about 1 GB", about: "Most languages, Tagalog too" };

export type Progress = { received: number; total: number };

/** The download state of the on-device model. */
export function useLocalModel() {
  const [ready, setReady] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.modelStatus().then(setReady).catch(() => {});
    const off = onModelDownload(setProgress);
    return () => {
      off.then((f) => f());
    };
  }, []);

  const download = async () => {
    setError(null);
    setDownloading(true);
    setProgress({ received: 0, total: 0 });
    try {
      await api.downloadModel();
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
