import { type FormEvent, useEffect, useState } from "react";
import { ConfirmDialog } from "../app/ConfirmDialog";
import { useLeaveGuard } from "../app/useLeaveGuard";
import { api, errorMessage, type KeyAccount, type Settings, type UsageTotal } from "../lib/api";
import {
  LOCAL_MODEL,
  type ModelDownload,
  type ModelInfo,
  NOTES_MODEL,
  SEARCH_MODEL,
  useLocalModel,
  useNotesModel,
  useSearchIndex,
  useSearchModel,
} from "../lib/localModel";
import { TIMING_CHOICES } from "../lib/transcribeTiming";
import "./note/note.css";

type KeyFieldProps = { account: KeyAccount; label: string };

/** A write-only key field. It shows if a key is saved, but never the key. */
function KeyField({ account, label }: KeyFieldProps) {
  const [value, setValue] = useState("");
  const [saved, setSaved] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    api.hasApiKey(account).then(setSaved).catch(() => {});
  }, [account]);

  const save = async () => {
    try {
      await api.setApiKey(account, value);
      setValue("");
      const has = await api.hasApiKey(account);
      setSaved(has);
      setMessage(has ? "Saved in the Keychain." : "The key is not saved.");
    } catch (e) {
      setMessage(errorMessage(e));
    }
  };

  // The backend deletes a key when it gets an empty value (issue #20).
  const remove = async () => {
    setConfirming(false);
    try {
      await api.setApiKey(account, "");
      setSaved(false);
      setMessage("The key is removed.");
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
        <button type="button" className="btn" onClick={save} disabled={!value.trim()}>Save key</button>
        {saved && <button type="button" className="btn btn-danger" onClick={() => setConfirming(true)}>Remove key</button>}
      </div>
      <span className="muted small">{message ?? (saved ? "A key is saved." : "No key is saved.")}</span>
      {confirming && (
        <ConfirmDialog
          title={`Remove the ${label}?`}
          body="Morsel deletes this key from the Keychain. You cannot undo this."
          confirmLabel="Remove key"
          onConfirm={remove}
          onCancel={() => setConfirming(false)}
        />
      )}
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
  const [saved, setSaved] = useState<Settings | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [keyMessage, setKeyMessage] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const model = useLocalModel();
  const notesModel = useNotesModel();
  const searchModel = useSearchModel();
  const searchIndex = useSearchIndex();
  const [usage, setUsage] = useState<UsageTotal | null>(null);

  useEffect(() => {
    api.usageSummary().then(setUsage).catch(() => {});
    api.getSettings().then((s) => {
      setSettings(s);
      setSaved(s);
    }).catch((e) => setMessage(errorMessage(e)));
  }, []);

  const dirty = JSON.stringify(settings) !== JSON.stringify(saved);
  const { leaveTo, leave, stay } = useLeaveGuard(dirty);

  if (!settings) return <p className="muted page">{message ?? "Loading the settings…"}</p>;

  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setSettings({ ...settings, [key]: value });
    // An edit makes "Settings saved." and the last key test out of date (#29).
    setMessage(null);
    setKeyMessage(null);
  };

  const save = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await api.saveSettings(settings);
      setSaved(settings);
      setMessage("Settings saved.");
    } catch (err) {
      setMessage(errorMessage(err));
    }
  };

  const testKey = async () => {
    setKeyMessage(null);
    setTesting(true);
    try {
      if (!(await api.hasApiKey("notes_api_key"))) {
        setKeyMessage("Add and save the notes API key first.");
        return;
      }
      // The backend tests the saved settings, so save the form first, as onboarding does (#29).
      await api.saveSettings(settings);
      setMessage("Settings saved.");
      await api.testNotesKey();
      setKeyMessage("The key works.");
    } catch (e) {
      setKeyMessage(errorMessage(e));
    } finally {
      setTesting(false);
    }
  };

  return (
    <form className="page" onSubmit={save}>
      <h1 className="serif page-title">Models and transcription</h1>

      <section aria-labelledby="notes-ai" className="settings-section">
        <h2 id="notes-ai" className="section-title">Notes AI</h2>
        <fieldset className="row">
          <legend className="sr-only">Notes model location</legend>
          <label><input type="radio" name="notes-mode" checked={settings.notesMode === "local"} onChange={() => set("notesMode", "local")} /> On this Mac (private, no key)</label>
          <label><input type="radio" name="notes-mode" checked={settings.notesMode !== "local"} onChange={() => set("notesMode", "cloud")} /> Cloud (your key)</label>
        </fieldset>
        {settings.notesMode === "local" ? (
          <LocalModelField model={notesModel} info={NOTES_MODEL} action="Download the notes model" />
        ) : (
          <>
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
              <button type="button" className="btn" onClick={testKey} disabled={testing}>Test key</button>
              {keyMessage && <span className="small">{keyMessage}</span>}
            </div>
          </>
        )}
        <LocalModelField model={searchModel} info={SEARCH_MODEL} action="Download the search model" />
        {searchIndex && <p className="muted small" style={{ margin: 0 }}>Search index: {searchIndex.done} of {searchIndex.total}</p>}
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
          <LocalModelField model={model} info={LOCAL_MODEL} action="Download the model" />
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
        <fieldset className="field">
          <legend>When to transcribe</legend>
          {TIMING_CHOICES.map((choice) => (
            <label key={choice.value}>
              <input type="radio" name="timing" checked={settings.transcribeTiming === choice.value} onChange={() => set("transcribeTiming", choice.value)} />{" "}
              {choice.title}: <span className="muted small">{choice.about}</span>
            </label>
          ))}
        </fieldset>
        <div className="field">
          <label htmlFor="language">Meeting language ("auto" detects it)</label>
          <input id="language" className="text-input" value={settings.language} onChange={(e) => set("language", e.target.value)} />
        </div>
      </section>

      <div className="row">
        <button type="submit" className="btn btn-primary">Save settings</button>
        {message && <span role="status" className="small">{message}</span>}
      </div>
      {leaveTo && (
        <ConfirmDialog
          title="Discard your changes?"
          body="Your settings are not saved."
          confirmLabel="Discard changes"
          onConfirm={leave}
          onCancel={stay}
        />
      )}
    </form>
  );
}

type LocalModelFieldProps = { model: ModelDownload; info: ModelInfo; action: string };

/** The download state of one on-device model, and its download button. */
function LocalModelField({ model, info, action }: LocalModelFieldProps) {
  const p = model.downloading ? model.progress : null;
  const percent = p && p.total > 0 ? Math.floor((p.received / p.total) * 100) : p ? 0 : null;
  return (
    <div className="field">
      <span>{info.name} model ({info.size}): {model.ready ? "ready" : "not downloaded"}</span>
      <span className="muted small">{info.about}</span>
      {!model.ready && percent === null && (
        <button type="button" className="btn" onClick={model.download}>{action}</button>
      )}
      {percent !== null && (
        <>
          <progress max={100} value={percent} />
          <span className="small">Downloading: {percent}%</span>
        </>
      )}
      {model.error && <span role="alert" className="error-text small">{model.error}</span>}
    </div>
  );
}
