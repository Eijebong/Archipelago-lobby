-- Your SQL goes here
CREATE TYPE VALIDATION_STATUS AS ENUM ('validated', 'manually_validated', 'unsupported', 'failed', 'unknown');
CREATE TYPE APWORLD AS (
    name TEXT,
    version TEXT
);

ALTER TABLE yamls ADD COLUMN validation_status VALIDATION_STATUS NOT NULL DEFAULT 'unknown'::VALIDATION_STATUS;
ALTER TABLE yamls ADD COLUMN apworlds APWORLD[] NOT NULL DEFAULT '{}';
ALTER TABLE yamls ADD COLUMN last_validation_time TIMESTAMP NOT NULL DEFAULT NOW();
ALTER TABLE yamls ADD COLUMN last_error TEXT;
