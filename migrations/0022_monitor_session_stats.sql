BEGIN;

SET search_path TO fod, public;

-- Jedna ograniczona migawka na aktywna sesje FOD.
-- Szczegolowe liczniki sa wersjonowane w JSONB.
CREATE TABLE IF NOT EXISTS monitor_session_stats (
    session_id BIGINT PRIMARY KEY REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    fod_version VARCHAR(32) NOT NULL,
    sample_seq BIGINT NOT NULL DEFAULT 0,
    sampled_at TIMESTAMP NOT NULL DEFAULT NOW(),
    payload_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_monitor_session_stats_sampled
    ON monitor_session_stats (sampled_at);

COMMIT;
