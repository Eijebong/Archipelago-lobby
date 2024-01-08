-- Your SQL goes here

CREATE TABLE rooms(
    id BINARY PRIMARY KEY NOT NULL,
    name VARCHAR NOT NULL,
    close_date DATETIME NOT NULL
);

CREATE TABLE yamls(
    id BINARY PRIMARY KEY NOT NULL,
    room_id BINARY NOT NULL,
    owner_id BINARY NOT NULL,
    content TEXT NOT NULL,
    player_name VARCHAR NOT NULL,
    game VARCHAR NOT NULL
);
