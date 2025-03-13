-- This file should undo anything in `up.sql`
ALTER TABLE rooms DROP COLUMN meta_file;
ALTER TABLE room_templates DROP COLUMN meta_file;
