CREATE TABLE review_presets (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    builtin_rules JSONB NOT NULL DEFAULT '[]'
);

CREATE TABLE review_preset_rules (
    id SERIAL PRIMARY KEY,
    preset_id INTEGER NOT NULL REFERENCES review_presets(id) ON DELETE CASCADE,
    rule JSONB NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    last_edited_by BIGINT,
    last_edited_at TIMESTAMPTZ
);

CREATE TABLE room_review_config (
    room_id UUID PRIMARY KEY,
    preset_id INTEGER NOT NULL REFERENCES review_presets(id) ON DELETE CASCADE
);

CREATE TABLE yaml_review_status (
    room_id UUID NOT NULL,
    yaml_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'unreviewed',
    changed_by BIGINT NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (room_id, yaml_id),
    CHECK (status IN ('unreviewed', 'reported', 'ok'))
);
