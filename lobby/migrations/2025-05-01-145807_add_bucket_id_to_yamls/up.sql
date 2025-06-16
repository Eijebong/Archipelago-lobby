-- Your SQL goes here
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
ALTER TABLE yamls ADD COLUMN bucket_id UUID NOT NULL DEFAULT uuid_generate_v4();
