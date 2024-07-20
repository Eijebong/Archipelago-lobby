-- Your SQL goes here

ALTER TABLE yamls DROP COLUMN owner_id;
ALTER TABLE yamls ADD COLUMN owner_id BIGINT DEFAULT -1 NOT NULL;

CREATE TABLE discord_users(
    id BIGINT PRIMARY KEY NOT NULL,
    username VARCHAR NOT NULL
);

INSERT INTO discord_users(id, username) VALUES(-1, 'Unknown user');
