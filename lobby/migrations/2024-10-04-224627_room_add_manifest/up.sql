-- Your SQL goes here
ALTER TABLE rooms ADD COLUMN manifest JSONB NOT NULL DEFAULT '{}'::jsonb;
