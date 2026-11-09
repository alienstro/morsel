import { useEffect, useState } from "react";
import { api, errorMessage, onModelDownload, type LocalModel } from "./api";

/** The facts that a user needs to pick an on-device speech model. */
export const LOCAL_MODELS: Record<LocalModel, { name: string; size: string; about: string }> = {
  parakeet: { name: "Parakeet", size: "about 670 MB", about: "Fast. 25 European languages, not Tagalog" },
  whisper_turbo: { name: "Whisper Turbo", size: "about 1 GB", about: "Slower. Most languages, Tagalog too" },
};

export type Progress = { received: number; total: number };

/** The download state of each on-device model. Morsel downloads one model at a time. */
export function useLocalModels() {
  const [ready, setReady] = useState<Record<LocalModel, boolean>>({ parakeet: false, whisper_turbo: false });
  const [downloading, setDownloading] = useState<LocalModel | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    for (const model of Object.keys(LOCAL_MODELS) as LocalModel[]) {
      api.modelStatus(model).then((ok) => setReady((r) => ({ ...r, [model]: ok }))).catch(() => {});
    }
    const off = onModelDownload(setProgress);
    return () => {
      off.then((f) => f());
    };
  }, []);

  const download = async (model: LocalModel) => {
    setError(null);
    setDownloading(model);
    setProgress({ received: 0, total: 0 });
    try {
      await api.downloadModel(model);
      setReady((r) => ({ ...r, [model]: true }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setDownloading(null);
      setProgress(null);
    }
  };

  return { ready, downloading, progress, error, download };
}
