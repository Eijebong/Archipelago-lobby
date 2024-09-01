-- This file should undo anything in `up.sql`
ALTER TABLE yamls DROP CONSTRAINT fk_room;
ALTER TABLE yamls ADD CONSTRAINT yamls_room_id_fkey FOREIGN KEY (room_id) REFERENCES rooms(id);
