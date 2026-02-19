ALTER TABLE yamls ADD COLUMN edited_content TEXT;
ALTER TABLE yamls ADD COLUMN last_edited_by BIGINT;
ALTER TABLE yamls ADD COLUMN last_edited_by_name VARCHAR;
ALTER TABLE yamls ADD COLUMN last_edited_at TIMESTAMP;
