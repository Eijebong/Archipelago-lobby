-- This file should undo anything in `up.sql`
ALTER TABLE rooms DROP COLUMN allow_invalid_yamls;
ALTER TABLE room_templates DROP COLUMN allow_invalid_yamls;
