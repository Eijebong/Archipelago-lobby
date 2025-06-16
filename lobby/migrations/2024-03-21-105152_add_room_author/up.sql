-- Your SQL goes here

ALTER TABLE rooms ADD COLUMN author_id BIGINT NOT NULL DEFAULT -1 REFERENCES discord_users(id);
