CREATE TABLE teams (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    guild_id BIGINT NOT NULL
);

CREATE TABLE team_members (
    team_id INT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL,
    username TEXT,
    role TEXT NOT NULL DEFAULT 'reviewer' CHECK (role IN ('viewer', 'reviewer', 'rule_editor', 'editor')),
    PRIMARY KEY (team_id, user_id)
);

CREATE TABLE team_rooms (
    team_id INT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    room_id UUID NOT NULL,
    PRIMARY KEY (team_id, room_id)
);

ALTER TABLE review_presets ADD COLUMN team_id INT REFERENCES teams(id) ON DELETE CASCADE;
