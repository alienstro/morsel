import { useNavigate } from "react-router";
import { api } from "../../lib/api";
import { CalendarPanel } from "../calendar/CalendarPanel";
import { OnboardingLayout } from "./OnboardingLayout";

/** Screen 1.3: the optional calendar link. Finish and Skip both end the first run. */
export function CalendarStep() {
  const navigate = useNavigate();

  const done = async () => {
    const settings = await api.getSettings();
    await api.saveSettings({ ...settings, onboarded: true });
    navigate("/", { replace: true });
  };

  return (
    <OnboardingLayout
      step={3}
      title="Connect your calendar"
      lead="Morsel uses your Composio account to read your meetings. You can skip this step and connect later in Settings."
      footer={
        <>
          <button type="button" className="btn" onClick={() => navigate("/welcome/2")}>Back</button>
          <button type="button" className="btn btn-primary" onClick={done}>Finish</button>
          <span className="spacer" />
          <button type="button" className="btn btn-link" onClick={done}>Skip for now</button>
        </>
      }
    >
      <CalendarPanel />
      <section className="onboard-section" aria-labelledby="ob-reads">
        <h2 id="ob-reads">What Morsel reads</h2>
        <p className="muted small" style={{ margin: 0 }}>
          The title, the time, the video link and the people of each event. Morsel does not read event descriptions or attachments, and it never changes your calendar.
        </p>
      </section>
    </OnboardingLayout>
  );
}
