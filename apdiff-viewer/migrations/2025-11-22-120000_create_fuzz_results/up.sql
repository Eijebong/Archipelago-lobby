CREATE TABLE fuzz_results (
    id BIGSERIAL PRIMARY KEY,
    world_name VARCHAR NOT NULL,
    version VARCHAR NOT NULL,
    checksum VARCHAR NOT NULL,

    total INTEGER NOT NULL,
    success INTEGER NOT NULL,
    failure INTEGER NOT NULL,
    timeout INTEGER NOT NULL,
    ignored INTEGER NOT NULL,

    task_id VARCHAR NOT NULL,
    pr_number INTEGER,
    extra_args VARCHAR,

    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_fuzz_results_world_name ON fuzz_results(world_name);
CREATE INDEX idx_fuzz_results_world_version ON fuzz_results(world_name, version);
CREATE INDEX idx_fuzz_results_pr_number ON fuzz_results(pr_number);
CREATE INDEX idx_fuzz_results_recorded_at ON fuzz_results(recorded_at DESC);
