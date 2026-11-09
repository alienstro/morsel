-- The timing that each meeting started with, and how far the after-mode job came.
ALTER TABLE meetings ADD COLUMN transcribe_timing TEXT NOT NULL DEFAULT 'live';
ALTER TABLE meetings ADD COLUMN transcribed_ms INTEGER NOT NULL DEFAULT 0;
