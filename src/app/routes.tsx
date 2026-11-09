import { useEffect, useState } from "react";
import { Navigate, Outlet, Route, Routes, useLocation, useNavigate } from "react-router";
import { api, onOpenMeeting } from "../lib/api";
import { MeetingScreen } from "../screens/meeting/MeetingScreen";
import { HomeScreen } from "../screens/HomeScreen";
import { NoteScreen } from "../screens/note/NoteScreen";
import { SettingsModelsScreen } from "../screens/SettingsModelsScreen";
import { WritingScreen } from "../screens/note/WritingScreen";
import { AskScreen } from "../screens/ask/AskScreen";
import { NotesScreen } from "../screens/notes/NotesScreen";
import { SettingsLayout, SettingsPending } from "../screens/settings/SettingsLayout";
import { SettingsCalendarScreen } from "../screens/settings/SettingsCalendarScreen";
import { SettingsPrivacyScreen } from "../screens/settings/SettingsPrivacyScreen";
import { TemplatesScreen } from "../screens/templates/TemplatesScreen";
import { OnboardingScreen } from "../screens/onboarding/OnboardingScreen";
import { AppShell, type RecentItem } from "./AppShell";

function ComingSoon() {
  return <p style={{ padding: 32 }} className="muted">Coming soon.</p>;
}

function ShellLayout() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const [recent, setRecent] = useState<RecentItem[]>([]);
  const [noteCount, setNoteCount] = useState(0);

  useEffect(() => {
    api.listMeetings(3).then((m) => setRecent(m.map(({ id, title }) => ({ id, title })))).catch(() => {});
    api.searchNotes({ limit: 0, offset: 0 }).then((p) => setNoteCount(p.total)).catch(() => {});
  }, [pathname]);

  return (
    <AppShell recent={recent} noteCount={noteCount} onNewNote={() => navigate("/")}>
      <Outlet />
    </AppShell>
  );
}

/** Opens a meeting that the reminder or the global shortcut started. */
function OpenMeetingListener() {
  const navigate = useNavigate();
  useEffect(() => {
    const off = onOpenMeeting(({ id }) => navigate(`/meeting/${id}`));
    return () => {
      off.then((f) => f());
    };
  }, [navigate]);
  return null;
}

/** Sends a new user to the first-run screens. Shows nothing until the settings load. */
function OnboardingGate() {
  const [onboarded, setOnboarded] = useState<boolean | null>(null);

  useEffect(() => {
    api.getSettings().then((s) => setOnboarded(s.onboarded)).catch(() => setOnboarded(true));
  }, []);

  if (onboarded === null) return null;
  return onboarded ? (
    <>
      <OpenMeetingListener />
      <Outlet />
    </>
  ) : (
    <Navigate to="/welcome/1" replace />
  );
}

/** Screen routes. The meeting screen has its own meeting list, so it renders without the app shell. */
export function AppRoutes() {
  return (
    <Routes>
      <Route path="/welcome/:step" element={<OnboardingScreen />} />
      <Route element={<OnboardingGate />}>
      <Route path="/meeting/:id" element={<MeetingScreen />} />
      <Route element={<ShellLayout />}>
        <Route path="/" element={<HomeScreen />} />
        <Route path="/meeting/:id/writing" element={<WritingScreen />} />
        <Route path="/note/:id" element={<NoteScreen />} />
        <Route path="/notes" element={<NotesScreen />} />
        <Route path="/templates" element={<TemplatesScreen />} />
        <Route path="/ask" element={<AskScreen />} />
        <Route path="/ask/:id" element={<AskScreen />} />
        <Route path="*" element={<ComingSoon />} />
      </Route>
      <Route element={<SettingsLayout />}>
        <Route path="/settings" element={<Navigate to="/settings/models" replace />} />
        <Route path="/settings/models" element={<SettingsModelsScreen />} />
        <Route path="/settings/privacy" element={<SettingsPrivacyScreen />} />
        <Route path="/settings/calendar" element={<SettingsCalendarScreen />} />
        <Route path="/settings/shortcuts" element={<SettingsPending title="Shortcuts" />} />
      </Route>
      </Route>
    </Routes>
  );
}
