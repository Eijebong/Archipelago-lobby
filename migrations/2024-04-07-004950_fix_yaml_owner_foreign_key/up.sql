-- Your SQL goes here

CREATE TEMPORARY TABLE temp AS SELECT * FROM yamls;
DROP TABLE yamls;

CREATE TABLE yamls(
    id BINARY PRIMARY KEY NOT NULL,
    room_id BINARY NOT NULL REFERENCES rooms(id),
    content TEXT NOT NULL,
    player_name VARCHAR NOT NULL,
    game VARCHAR NOT NULL,
    owner_id BIGINT DEFAULT -1 NOT NULL REFERENCES discord_users(id)
);

INSERT INTO yamls
 (
    id,
    room_id,
    content,
    player_name,
    game,
    owner_id
)
SELECT
    id,
    room_id,
    content,
    player_name,
    game,
    owner_id
FROM temp;

