CREATE TABLE yaml_review_notes (
    id SERIAL PRIMARY KEY,
    room_id UUID NOT NULL,
    yaml_id UUID NOT NULL,
    content TEXT NOT NULL,
    author_id BIGINT NOT NULL,
    author_name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_yaml_review_notes_room_yaml ON yaml_review_notes (room_id, yaml_id);
