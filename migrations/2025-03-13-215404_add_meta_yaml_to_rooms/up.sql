-- Your SQL goes here
ALTER TABLE rooms ADD COLUMN meta_file TEXT NOT NULL DEFAULT '';
ALTER TABLE room_templates ADD COLUMN meta_file TEXT NOT NULL DEFAULT '';
