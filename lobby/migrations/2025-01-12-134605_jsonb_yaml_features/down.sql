-- This file should undo anything in `up.sql`
ALTER TABLE yamls ALTER COLUMN features TYPE JSON;
