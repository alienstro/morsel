import { useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { api, type MicAccess } from "../../lib/api";
import { OnboardingLayout } from "./OnboardingLayout";

function Allowed() {
  return (
    <span className="perm-ok">
      <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round"><path d="M5 12l5 5 9-10" /></svg>
      Allowed
    </span>
  );
}

/** Screen 1.1: the welcome text and the two audio permissions. */
export function WelcomeStep() {
  const navigate = useNavigate();
  const [mic, setMic] = useState<MicAccess>("unknown");

  useEffect(() => {
    api.microphoneAccess().then(setMic).catch(() => {});
  }, []);

  const askMic = async () => setMic(await api.requestMicrophone());

  return (
    <OnboardingLayout
      step={1}
      title="Welcome to Morsel"
      lead="Morsel writes your meeting notes from what you type and what it hears. It never joins the call, and it records only when you start a note."
      footer={
        <>
          <button type="button" className="btn btn-primary" disabled={mic !== "allowed"} onClick={() => navigate("/welcome/2")}>Continue</button>
          {mic !== "allowed" && <span className="muted small">Allow the microphone to continue.</span>}
        </>
      }
    >
      <div className="perm-list">
        <div className="perm-row">
          <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5 11a7 7 0 0 0 14 0M12 18v3" /></svg>
          <div className="perm-text">
            <div className="perm-name">Microphone</div>
            <div className="muted small">Hears your voice</div>
          </div>
          {mic === "allowed" && <Allowed />}
          {mic === "unknown" && <button type="button" className="btn" aria-label="Allow the microphone" onClick={askMic}>Allow</button>}
          {mic === "denied" && (
            <button type="button" className="btn" aria-label="Open System Settings for the microphone" onClick={() => api.openPrivacySettings("microphone")}>Open System Settings</button>
          )}
        </div>
        <div className="perm-row">
          <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"><path d="M4 9v6h4l5 4V5L8 9z" /><path d="M16 9a4 4 0 0 1 0 6M19 6a8 8 0 0 1 0 12" /></svg>
          <div className="perm-text">
            <div className="perm-name">System audio</div>
            <div className="muted small">Hears the other people in the call</div>
          </div>
          <button type="button" className="btn" aria-label="Open System Settings for system audio" onClick={() => api.openPrivacySettings("audio_capture")}>Open System Settings</button>
        </div>
      </div>
      <p className="muted small" style={{ margin: 0 }}>
        macOS lists the system audio permission as "Screen &amp; System Audio Recording". Morsel does not record your screen. macOS asks for it the first time you start a note.
      </p>
    </OnboardingLayout>
  );
}
