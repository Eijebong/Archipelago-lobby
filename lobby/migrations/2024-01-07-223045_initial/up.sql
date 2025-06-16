-- Your SQL goes here

CREATE TABLE rooms(
    id UUID PRIMARY KEY NOT NULL,
    name VARCHAR NOT NULL,
    close_date TIMESTAMP NOT NULL
);

CREATE TABLE yamls(
    id UUID PRIMARY KEY NOT NULL,
    room_id UUID NOT NULL,
    owner_id UUID NOT NULL,
    content TEXT NOT NULL,
    player_name VARCHAR NOT NULL,
    game VARCHAR NOT NULL
);
