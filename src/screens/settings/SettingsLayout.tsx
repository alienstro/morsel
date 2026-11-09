import { Link, NavLink, Outlet } from "react-router";
import "../library.css";

/** The settings shell (board 5.2): a settings sidebar in place of the main one. */
export function SettingsLayout() {
  return (
    <div className="shell">
      <div className="titlebar" data-tauri-drag-region>
        Audio stays on this Mac
      </div>
      <div className="shell-body">
        <nav aria-label="Settings" className="sidebar">
          <Link to="/" className="nav-link muted">
            <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round"><path d="M15 6l-6 6 6 6" /></svg>
            Back to Morsel
          </Link>
          <div className="nav-group">
            <div className="nav-label">Settings</div>
            <NavLink to="/settings/models" end className="nav-link">Models</NavLink>
            <Link to="/settings/models#transcription" className="nav-link">Transcription</Link>
            <NavLink to="/settings/calendar" className="nav-link">Calendar</NavLink>
            <NavLink to="/settings/privacy" className="nav-link">Privacy and data</NavLink>
            <NavLink to="/settings/shortcuts" className="nav-link">Shortcuts</NavLink>
          </div>
        </nav>
        <div className="content">
          <div className="surface">
            <Outlet />
          </div>
        </div>
      </div>
    </div>
  );
}
