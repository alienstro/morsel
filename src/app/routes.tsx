import { useEffect, useState } from "react";
import { Outlet, Route, Routes, useLocation, useNavigate } from "react-router";
import { api } from "../lib/api";
import { MeetingScreen } from "../screens/meeting/MeetingScreen";
import { HomeScreen } from "../screens/HomeScreen";
import { NoteScreen } from "../screens/note/NoteScreen";
import { SettingsModelsScreen } from "../screens/SettingsModelsScreen";
import { WritingScreen } from "../screens/note/WritingScreen";
import { AskScreen } from "../screens/ask/AskScreen";
import { NotesScreen } from "../screens/notes/NotesScreen";
import { TemplatesScreen } from "../screens/templates/TemplatesScreen";
import { AppShell, type RecentItem } from "./AppShell";

function ComingSoon() {
  return <p style={{ padding: 32 }} className="muted">Coming soon.</p>;
}

function ShellLayout() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const [recent, setRecent] = useState<RecentItem[]>([]);

  useEffect(() => {
    api.listMeetings(3).then((m) => setRecent(m.map(({ id, title }) => ({ id, title })))).catch(() => {});
  }, [pathname]);

  return (
    <AppShell recent={recent} onNewNote={() => navigate("/")}>
      <Outlet />
    </AppShell>
  );
}

/** Screen routes. The meeting screen has its own meeting list, so it renders without the app shell. */
export function AppRoutes() {
  return (
    <Routes>
      <Route path="/meeting/:id" element={<MeetingScreen />} />
      <Route element={<ShellLayout />}>
        <Route path="/" element={<HomeScreen />} />
        <Route path="/meeting/:id/writing" element={<WritingScreen />} />
        <Route path="/note/:id" element={<NoteScreen />} />
        <Route path="/notes" element={<NotesScreen />} />
        <Route path="/templates" element={<TemplatesScreen />} />
        <Route path="/ask" element={<AskScreen />} />
        <Route path="/ask/:id" element={<AskScreen />} />
        <Route path="/settings/models" element={<SettingsModelsScreen />} />
        <Route path="*" element={<ComingSoon />} />
      </Route>
    </Routes>
  );
}
