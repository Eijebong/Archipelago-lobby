-- This file should undo anything in `up.sql`
ALTER TABLE yamls RENAME COLUMN bundle_id TO bucket_id;
