-- Your SQL goes here
ALTER TABLE yamls ADD COLUMN features JSON NOT NULL DEFAULT '{}';
