import type { ReactNode } from "react";
import { NavLink } from "react-router";

export type RecentItem = { id: number; title: string };

type Props = {
  recent?: RecentItem[];
  onNewNote?: () => void;
  children: ReactNode;
};

const stroke = { fill: "none", stroke: "currentColor", strokeWidth: 1.8 } as const;

export function AppShell({ recent = [], onNewNote, children }: Props) {
  return (
    <div className="shell">
      <div className="titlebar" data-tauri-drag-region>
        Audio stays on this Mac
      </div>
      <div className="shell-body">
        <nav aria-label="Main" className="sidebar">
          <div className="sidebar-head">
            <span className="brand">Morsel</span>
            <button type="button" className="icon-btn" aria-label="New note" onClick={onNewNote}>
              <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" {...stroke} strokeWidth={2.2} strokeLinecap="round">
                <path d="M12 5v14M5 12h14" />
              </svg>
            </button>
          </div>
          <div className="nav-group">
            <NavLink to="/" end className="nav-link">
              <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" {...stroke}><path d="M4 11l8-6 8 6v9H4z" /></svg>
              Home
            </NavLink>
            <NavLink to="/notes" className="nav-link">
              <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" {...stroke}><path d="M6 3h9l4 4v14H6z" /><path d="M9 12h7M9 16h5" /></svg>
              Notes
            </NavLink>
            <NavLink to="/templates" className="nav-link">
              <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" {...stroke}><rect x="4" y="4" width="16" height="16" rx="2" /><path d="M4 10h16M10 10v10" /></svg>
              Templates
            </NavLink>
            <NavLink to="/ask" className="nav-link">
              <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" {...stroke}><path d="M4 5h16v11H9l-5 4z" /></svg>
              Ask Morsel
            </NavLink>
          </div>
          {recent.length > 0 && (
            <div className="nav-group">
              <div className="nav-label">Recent</div>
              {recent.map((item) => (
                <NavLink key={item.id} to={`/note/${item.id}`} className="nav-link">
                  <span className="truncate">{item.title}</span>
                </NavLink>
              ))}
            </div>
          )}
          <span style={{ flexGrow: 1 }} />
          <NavLink to="/settings/models" className="nav-link muted">
            <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" {...stroke} strokeLinecap="round"><path d="M4 7h10M18 7h2M4 17h4M12 17h8" /><circle cx="16" cy="7" r="2" /><circle cx="10" cy="17" r="2" /></svg>
            Settings
          </NavLink>
        </nav>
        <div className="content">
          <div className="surface">{children}</div>
        </div>
      </div>
    </div>
  );
}
