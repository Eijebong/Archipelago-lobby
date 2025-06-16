-- Your SQL goes here
ALTER TABLE rooms ADD COLUMN yaml_limit_bypass_list BIGINT[] NOT NULL DEFAULT '{}';
