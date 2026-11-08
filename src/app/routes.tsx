import { Outlet, Route, Routes } from "react-router";
import { MeetingScreen } from "../screens/meeting/MeetingScreen";
import { AppShell } from "./AppShell";

function ComingSoon() {
  return <p style={{ padding: 32 }} className="muted">Coming soon.</p>;
}

function ShellLayout() {
  return (
    <AppShell>
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
        <Route path="/" element={<ComingSoon />} />
        <Route path="/meeting/:id/writing" element={<ComingSoon />} />
        <Route path="/note/:id" element={<ComingSoon />} />
        <Route path="/settings/models" element={<ComingSoon />} />
        <Route path="*" element={<ComingSoon />} />
      </Route>
    </Routes>
  );
}
