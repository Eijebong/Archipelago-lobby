-- Your SQL goes here
ALTER TABLE rooms ADD COLUMN allow_invalid_yamls BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE room_templates ADD COLUMN allow_invalid_yamls BOOLEAN NOT NULL DEFAULT FALSE;
