-- Your SQL goes here

CREATE INDEX yamls_rooms_owner ON yamls(room_id, owner_id);
CREATE INDEX rooms_close_date ON rooms(close_date);
