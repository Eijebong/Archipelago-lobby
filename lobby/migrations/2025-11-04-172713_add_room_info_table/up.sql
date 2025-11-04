-- Your SQL goes here

CREATE TABLE room_info(
    room_id UUID PRIMARY KEY NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    host VARCHAR NOT NULL,
    port INTEGER NOT NULL
);
