import { useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { api, errorMessage, type Settings } from "../../lib/api";
import { downloadStateText, LOCAL_MODEL, NOTES_MODEL, useLocalModel, useNotesModel } from "../../lib/localModel";
import { TIMING_CHOICES } from "../../lib/transcribeTiming";
import { OnboardingLayout } from "./OnboardingLayout";

const OPENROUTER = "https://openrouter.ai/api/v1";

/** Screen 1.2: the transcription choice and the notes model. */
export function AiStep() {
  const navigate = useNavigate();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [notesKey, setNotesKey] = useState("");
  const [groqKey, setGroqKey] = useState("");
  const model = useLocalModel();
  const notesModel = useNotesModel();
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.getSettings().then(setSettings).catch((e) => setMessage({ ok: false, text: errorMessage(e) }));
  }, []);

  if (!settings) return null;

  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => setSettings({ ...settings, [key]: value });
  const provider = settings.notesBaseUrl === OPENROUTER ? "openrouter" : "custom";

  /** Saves the settings and the keys that the user typed. Keys go only to the Keychain. */
  const persist = async () => {
    await api.saveSettings(settings);
    if (notesKey.trim()) await api.setApiKey("notes_api_key", notesKey.trim());
    if (groqKey.trim()) await api.setApiKey("transcribe_api_key", groqKey.trim());
  };

  const test = async () => {
    setMessage(null);
    setBusy(true);
    try {
      await persist();
      await api.testNotesKey();
      setMessage({ ok: true, text: "Connected" });
    } catch (e) {
      setMessage({ ok: false, text: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const next = async () => {
    try {
      await persist();
      navigate("/welcome/3");
    } catch (e) {
      setMessage({ ok: false, text: errorMessage(e) });
    }
  };

  const local = settings.transcribeMode === "local";
  const state = downloadStateText(model);
  const localNotes = settings.notesMode !== "cloud";
  const notesState = downloadStateText(notesModel);

  return (
    <OnboardingLayout
      step={2}
      title="Set up the AI"
      lead="You use your own keys. Morsel has no account and no bill of its own."
      footer={
        <>
          <button type="button" className="btn" onClick={() => navigate("/welcome/1")}>Back</button>
          <button type="button" className="btn btn-primary" onClick={next}>Continue</button>
        </>
      }
    >
      <section className="onboard-section" aria-labelledby="ob-transcription">
        <h2 id="ob-transcription">Transcription</h2>
        <div className="choice-list" role="radiogroup" aria-labelledby="ob-transcription">
          <label className={`choice${local ? " selected" : ""}`}>
            <input type="radio" name="tx" checked={local} onChange={() => set("transcribeMode", "local")} />
            <span>
              <span className="choice-title">On this Mac: {LOCAL_MODEL.name}</span>
              <span className="muted small" style={{ display: "block" }}>
                Free, audio never leaves your Mac, {LOCAL_MODEL.size}. {LOCAL_MODEL.about}
              </span>
              {local && (state
                ? <span className="small" style={{ display: "block" }}>{state}</span>
                : <button type="button" className="btn" style={{ marginTop: 6 }} onClick={model.download}>Download the model</button>)}
              {local && model.error && <span role="alert" className="error-text small" style={{ display: "block" }}>{model.error}</span>}
            </span>
          </label>
          <label className={`choice${settings.transcribeMode === "cloud" ? " selected" : ""}`}>
            <input type="radio" name="tx" checked={settings.transcribeMode === "cloud"} onChange={() => set("transcribeMode", "cloud")} />
            <span>
              <span className="choice-title">Cloud</span>
              <span className="muted small" style={{ display: "block" }}>Groq Whisper Turbo, about $0.04 for each hour, needs a Groq key. Most languages, Tagalog too</span>
            </span>
          </label>
        </div>
        {settings.transcribeMode === "cloud" && (
          <div className="field">
            <label htmlFor="ob-groq">Groq key</label>
            <input id="ob-groq" type="password" className="text-input" autoComplete="off" value={groqKey} onChange={(e) => setGroqKey(e.target.value)} />
          </div>
        )}
      </section>

      <section className="onboard-section" aria-labelledby="ob-timing">
        <h2 id="ob-timing">When to transcribe</h2>
        <div className="choice-list" role="radiogroup" aria-labelledby="ob-timing">
          {TIMING_CHOICES.map((choice) => {
            const selected = settings.transcribeTiming === choice.value;
            return (
              <label key={choice.value} className={`choice${selected ? " selected" : ""}`}>
                <input type="radio" name="timing" checked={selected} onChange={() => set("transcribeTiming", choice.value)} />
                <span>
                  <span className="choice-title">{choice.title}</span>
                  <span className="muted small" style={{ display: "block" }}>{choice.about}</span>
                </span>
              </label>
            );
          })}
        </div>
      </section>

      <section className="onboard-section" aria-labelledby="ob-notes">
        <h2 id="ob-notes">Notes model</h2>
        <div className="choice-list" role="radiogroup" aria-labelledby="ob-notes">
          <label className={`choice${localNotes ? " selected" : ""}`}>
            <input type="radio" name="notes" checked={localNotes} onChange={() => set("notesMode", "local")} />
            <span>
              <span className="choice-title">On this Mac: {NOTES_MODEL.name}</span>
              <span className="muted small" style={{ display: "block" }}>
                Free, private, {NOTES_MODEL.size}. {NOTES_MODEL.about}
              </span>
              {localNotes && (notesState
                ? <span className="small" style={{ display: "block" }}>{notesState}</span>
                : <button type="button" className="btn" style={{ marginTop: 6 }} onClick={notesModel.download}>Download the notes model</button>)}
              {localNotes && notesModel.error && <span role="alert" className="error-text small" style={{ display: "block" }}>{notesModel.error}</span>}
            </span>
          </label>
          <label className={`choice${!localNotes ? " selected" : ""}`}>
            <input type="radio" name="notes" checked={!localNotes} onChange={() => set("notesMode", "cloud")} />
            <span>
              <span className="choice-title">Cloud</span>
              <span className="muted small" style={{ display: "block" }}>OpenRouter or any OpenAI-compatible API, with your own key</span>
            </span>
          </label>
        </div>
        {!localNotes && (
          <>
            <div className="field">
              <label htmlFor="ob-provider">Provider</label>
              <select id="ob-provider" value={provider} onChange={(e) => set("notesBaseUrl", e.target.value === "openrouter" ? OPENROUTER : "")}>
                <option value="openrouter">OpenRouter</option>
                <option value="custom">OpenAI-compatible (custom URL)</option>
              </select>
            </div>
            {provider === "custom" && (
              <div className="field">
                <label htmlFor="ob-base">Base URL</label>
                <input id="ob-base" className="text-input" placeholder="https://…/v1" value={settings.notesBaseUrl} onChange={(e) => set("notesBaseUrl", e.target.value)} />
              </div>
            )}
            <div className="field">
              <label htmlFor="ob-key">API key</label>
              <input id="ob-key" type="password" className="text-input" autoComplete="off" value={notesKey} onChange={(e) => setNotesKey(e.target.value)} />
            </div>
            <div className="field">
              <label htmlFor="ob-model">Model</label>
              <input id="ob-model" className="text-input" placeholder="For example: provider/model-name" value={settings.notesModel} onChange={(e) => set("notesModel", e.target.value)} />
            </div>
            <div className="row">
              <button type="button" className="btn" onClick={test} disabled={busy}>Test connection</button>
              {message && <span role={message.ok ? "status" : "alert"} className={message.ok ? "perm-ok" : "error-text small"}>{message.text}</span>}
            </div>
            <p className="muted small" style={{ margin: 0 }}>Morsel keeps your key in the macOS Keychain.</p>
          </>
        )}
      </section>
    </OnboardingLayout>
  );
}
