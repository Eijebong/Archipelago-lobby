-- This file should undo anything in `up.sql`

ALTER TABLE yamls DROP COLUMN owner_id;
ALTER TABLE yamls ADD COLUMN owner_id BINARY NOT NULL DEFAULT 0;

DROP TABLE discord_users;
