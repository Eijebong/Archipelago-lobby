-- Your SQL goes here
ALTER TABLE yamls DROP CONSTRAINT yamls_room_id_fkey;
ALTER TABLE yamls ADD CONSTRAINT fk_room FOREIGN KEY (room_id) REFERENCES rooms(id) ON DELETE CASCADE;
