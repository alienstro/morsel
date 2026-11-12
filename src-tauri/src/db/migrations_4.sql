-- The parts of each meeting that Ask searches. The embedding is 768 little-endian f32 values.
CREATE TABLE chunks (
  id INTEGER PRIMARY KEY,
  meeting_id INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  part INTEGER NOT NULL,
  start_ms INTEGER,
  end_ms INTEGER,
  text TEXT NOT NULL,
  embedding BLOB
);
CREATE INDEX chunks_meeting ON chunks(meeting_id, part);

-- The keyword index of the part text. The triggers keep it in step with `chunks`.
CREATE VIRTUAL TABLE chunks_fts USING fts5(
  text,
  content = 'chunks',
  content_rowid = 'id',
  tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER chunks_after_insert AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER chunks_after_delete AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER chunks_after_text_update AFTER UPDATE OF text ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;
