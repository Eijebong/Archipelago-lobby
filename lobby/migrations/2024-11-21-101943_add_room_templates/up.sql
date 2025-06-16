CREATE TABLE room_templates(
    id UUID PRIMARY KEY NOT NULL,

    -- templatable ROOM fields
    name VARCHAR NOT NULL,
    close_date TIMESTAMP  NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    room_url VARCHAR NOT NULL DEFAULT '',
    author_id BIGINT NOT NULL REFERENCES discord_users(id),
    yaml_validation BOOLEAN NOT NULL DEFAULT TRUE,
    allow_unsupported BOOLEAN NOT NULL DEFAULT FALSE,
    yaml_limit_per_user integer,
    yaml_limit_bypass_list BIGINT[] NOT NULL DEFAULT '{}',
    manifest JSONB NOT NULL DEFAULT '{}'::jsonb,
    show_apworlds BOOLEAN NOT NULL DEFAULT TRUE
);

