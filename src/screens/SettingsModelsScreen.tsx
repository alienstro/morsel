import { type FormEvent, useEffect, useState } from "react";
import { api, errorMessage, onModelDownload, type KeyAccount, type Settings, type UsageTotal } from "../lib/api";
import "./note/note.css";

type KeyFieldProps = { account: KeyAccount; label: string };

/** A write-only key field. It shows if a key is saved, but never the key. */
function KeyField({ account, label }: KeyFieldProps) {
  const [value, setValue] = useState("");
  const [saved, setSaved] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    api.hasApiKey(account).then(setSaved).catch(() => {});
  }, [account]);

  const save = async () => {
    try {
      await api.setApiKey(account, value);
      setValue("");
      const has = await api.hasApiKey(account);
      setSaved(has);
      setMessage(has ? "Saved in the Keychain." : "The key is deleted.");
    } catch (e) {
      setMessage(errorMessage(e));
    }
  };

  return (
    <div className="field">
      <label htmlFor={account}>{label}</label>
      <div className="row">
        <input id={account} type="password" autoComplete="off" className="text-input" value={value}
          placeholder={saved ? "Saved. Type a new key to replace it." : "Paste your key"} onChange={(e) => setValue(e.target.value)} />
        <button type="button" className="btn" onClick={save}>Save key</button>
      </div>
      <span className="muted small">{message ?? (saved ? "A key is saved." : "No key is saved.")}</span>
    </div>
  );
}

/** "Usage, last 30 days: 1,500 tokens", with ", $0.04" only when a provider returned a cost. */
function usageText(u: UsageTotal): string {
  const tokens = (u.promptTokens + u.completionTokens).toLocaleString("en-US");
  const cost = u.cost === null ? "" : `, $${u.cost.toFixed(2)}`;
  return `Usage, last 30 days: ${tokens} tokens${cost}`;
}

export function SettingsModelsScreen() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [keyMessage, setKeyMessage] = useState<string | null>(null);
  const [modelReady, setModelReady] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [usage, setUsage] = useState<UsageTotal | null>(null);

  useEffect(() => {
    api.usageSummary().then(setUsage).catch(() => {});
    api.getSettings().then(setSettings).catch((e) => setMessage(errorMessage(e)));
    api.modelStatus().then(setModelReady).catch(() => {});
    const off = onModelDownload(({ received, total }) => total > 0 && setProgress(Math.floor((received / total) * 100)));
    return () => {
      off.then((f) => f());
    };
  }, []);

  if (!settings) return <p className="muted page">{message ?? "Loading the settings…"}</p>;

  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => setSettings({ ...settings, [key]: value });

  const save = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await api.saveSettings(settings);
      setMessage("Settings saved.");
    } catch (err) {
      setMessage(errorMessage(err));
    }
  };

  const testKey = async () => {
    setKeyMessage(null);
    try {
      await api.testNotesKey();
      setKeyMessage("The key works.");
    } catch (e) {
      setKeyMessage(errorMessage(e));
    }
  };

  const download = async () => {
    setDownloadError(null);
    setProgress(0);
    try {
      await api.downloadModel();
      setModelReady(true);
      setProgress(null);
    } catch (e) {
      setDownloadError(errorMessage(e));
      setProgress(null);
    }
  };

  return (
    <form className="page" onSubmit={save}>
      <h1 className="serif page-title">Models and transcription</h1>

      <section aria-labelledby="notes-ai" className="settings-section">
        <h2 id="notes-ai" className="section-title">Notes AI</h2>
        <div className="field">
          <label htmlFor="notes-base">Base URL (OpenRouter or any OpenAI-compatible API)</label>
          <input id="notes-base" className="text-input" value={settings.notesBaseUrl} onChange={(e) => set("notesBaseUrl", e.target.value)} />
        </div>
        <div className="field">
          <label htmlFor="notes-model">Notes model</label>
          <input id="notes-model" className="text-input" placeholder="For example: provider/model-name" value={settings.notesModel} onChange={(e) => set("notesModel", e.target.value)} />
        </div>
        <div className="field">
          <label htmlFor="quick-model">Quick model (titles and short questions; empty uses the notes model)</label>
          <input id="quick-model" className="text-input" value={settings.quickModel} onChange={(e) => set("quickModel", e.target.value)} />
        </div>
        <KeyField account="notes_api_key" label="Notes API key" />
        <div className="row">
          <button type="button" className="btn" onClick={testKey}>Test key</button>
          {keyMessage && <span className="small">{keyMessage}</span>}
        </div>
        {usage && <p className="muted small" style={{ margin: 0 }}>{usageText(usage)}</p>}
      </section>

      <section id="transcription" aria-labelledby="transcription-title" className="settings-section">
        <h2 id="transcription-title" className="section-title">Transcription</h2>
        <fieldset className="row">
          <legend className="sr-only">Transcription mode</legend>
          <label><input type="radio" name="mode" checked={settings.transcribeMode === "local"} onChange={() => set("transcribeMode", "local")} /> On this Mac (free, private)</label>
          <label><input type="radio" name="mode" checked={settings.transcribeMode === "cloud"} onChange={() => set("transcribeMode", "cloud")} /> Cloud (your key)</label>
        </fieldset>
        {settings.transcribeMode === "local" ? (
          <div className="field">
            <span>Parakeet model (about 670 MB): {modelReady ? "ready" : "not downloaded"}</span>
            {!modelReady && progress === null && (
              <button type="button" className="btn" onClick={download}>Download the model</button>
            )}
            {progress !== null && (
              <>
                <progress max={100} value={progress} />
                <span className="small">Downloading: {progress}%</span>
              </>
            )}
            {downloadError && <span role="alert" className="error-text small">{downloadError}</span>}
          </div>
        ) : (
          <>
            <div className="field">
              <label htmlFor="tx-base">Base URL</label>
              <input id="tx-base" className="text-input" value={settings.transcribeBaseUrl} onChange={(e) => set("transcribeBaseUrl", e.target.value)} />
            </div>
            <div className="field">
              <label htmlFor="tx-model">Model</label>
              <input id="tx-model" className="text-input" value={settings.transcribeModel} onChange={(e) => set("transcribeModel", e.target.value)} />
            </div>
            <KeyField account="transcribe_api_key" label="Transcription API key" />
          </>
        )}
        <div className="field">
          <label htmlFor="language">Meeting language ("auto" detects it)</label>
          <input id="language" className="text-input" value={settings.language} onChange={(e) => set("language", e.target.value)} />
        </div>
      </section>

      <div className="row">
        <button type="submit" className="btn btn-primary">Save settings</button>
        {message && <span role="status" className="small">{message}</span>}
      </div>
    </form>
  );
}
