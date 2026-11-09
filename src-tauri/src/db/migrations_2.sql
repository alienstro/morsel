ALTER TABLE templates ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE meetings ADD COLUMN people TEXT NOT NULL DEFAULT '[]';

-- One row for each meeting. The rowid is the meeting id.
CREATE VIRTUAL TABLE meeting_search USING fts5(
  title, user_notes, note, transcript,
  tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE asks (
  id INTEGER PRIMARY KEY,
  question TEXT NOT NULL,
  answer TEXT NOT NULL,
  sources_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

INSERT INTO meeting_search (rowid, title, user_notes, note, transcript)
SELECT m.id, m.title, COALESCE(u.body, ''), COALESCE(n.body_md, ''),
       COALESCE((SELECT group_concat(text, char(10)) FROM
                   (SELECT text FROM transcript_lines t WHERE t.meeting_id = m.id ORDER BY start_ms, id)), '')
FROM meetings m
LEFT JOIN user_notes u ON u.meeting_id = m.id
LEFT JOIN notes n ON n.meeting_id = m.id;
