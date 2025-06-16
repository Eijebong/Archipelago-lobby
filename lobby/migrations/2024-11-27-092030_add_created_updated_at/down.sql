-- This file should undo anything in `up.sql`

ALTER TABLE rooms DROP COLUMN created_at;
ALTER TABLE yamls DROP COLUMN created_at;
ALTER TABLE room_templates DROP COLUMN created_at;

ALTER TABLE rooms DROP COLUMN updated_at;
ALTER TABLE yamls DROP COLUMN updated_at;
ALTER TABLE room_templates DROP COLUMN updated_at;

DROP TRIGGER set_updated_at ON rooms;
DROP TRIGGER set_updated_at ON yamls;
DROP TRIGGER set_updated_at ON room_templates;

DROP FUNCTION set_updated_at;

