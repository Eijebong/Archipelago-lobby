-- This file should undo anything in `up.sql`
ALTER TABLE rooms DROP COLUMN is_bundle_room;
ALTER TABLE room_templates DROP COLUMN is_bundle_room;
