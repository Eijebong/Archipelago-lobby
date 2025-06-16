-- Your SQL goes here
CREATE TABLE generations (
    room_id UUID NOT NULL PRIMARY KEY,
    job_id UUID NOT NULL,
    status VARCHAR NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    FOREIGN KEY (room_id) REFERENCES rooms(id) ON DELETE CASCADE
);

