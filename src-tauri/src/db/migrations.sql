CREATE TABLE templates (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  sections_json TEXT NOT NULL,
  instructions TEXT NOT NULL,
  built_in INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE meetings (
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  template_id INTEGER NOT NULL REFERENCES templates(id),
  status TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  ended_at INTEGER
);

CREATE TABLE transcript_lines (
  id INTEGER PRIMARY KEY,
  meeting_id INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  speaker TEXT NOT NULL,
  start_ms INTEGER NOT NULL,
  end_ms INTEGER NOT NULL,
  text TEXT NOT NULL
);
CREATE INDEX transcript_lines_meeting ON transcript_lines(meeting_id, start_ms);

CREATE TABLE user_notes (
  meeting_id INTEGER PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
  body TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE notes (
  meeting_id INTEGER PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
  body_md TEXT NOT NULL,
  model TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE usage (
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  model TEXT NOT NULL,
  prompt_tokens INTEGER NOT NULL,
  completion_tokens INTEGER NOT NULL,
  cost REAL
);

INSERT INTO templates (id, name, sections_json, instructions, built_in) VALUES
  (1, 'General', '["Summary","Key points","Decisions","Action items"]',
   'Write short, clear notes for any meeting.', 1),
  (2, '1:1', '["Updates","Blockers","Feedback","Action items"]',
   'Focus on the progress, the problems and the support that each person needs.', 1),
  (3, 'Sales call', '["Pain points","Budget","Decision process","Next steps"]',
   'Focus on the needs of the customer, the money, who decides, and the next step.', 1),
  (4, 'Stand-up', '["Yesterday","Today","Blockers"]',
   'Group the points by person when the transcript shows who said them.', 1),
  (5, 'Product sync', '["Updates","Decisions","Risks","Action items"]',
   'Focus on the product decisions, the dates, and the risks.', 1);
